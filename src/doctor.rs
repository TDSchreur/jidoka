//! `factory doctor`: a report on whether this project and this machine are ready for the factory.
//! It only reads files, looks on `PATH` and asks `git` read-only questions, asks the pull request host's tool
//! whether it is signed in (`gh auth status`, `az extension show`, `az account show`), asks `rustup target list --installed`
//! in a Rust project on a non-Windows host (read-only), and, when a model-routing policy exists, asks an agent for its `--version`. It never runs the project's own commands, calls a model,
//! installs anything or needs the dashboard. The one thing it writes is a starter model-routing policy, and
//! only when `--create-model-routing` asks for it.

use crate::install::{bundled_skill_files, claude_config_dirs, skills_root};
use crate::routing::{install_starter, policy_path_in, RoutingAgent, Starter};
use serde::Serialize;
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

mod provider;
mod routing;

pub(crate) const AGENTS: [&str; 3] = ["codex", "claude", "copilot"];
/// Project files are not trusted: anything larger is not read.
const MAX_DOC_BYTES: u64 = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Pass,
    /// Something the user can supply: a file, a tool, an installed skill.
    Missing,
    /// Something is broken or cannot be read.
    Fail,
    /// A risk to read about that does not stop the factory, such as model routing that will not apply.
    Warning,
    /// Not looked at, with the reason, rather than reported as fine.
    Skipped,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Missing => "missing",
            Status::Fail => "fail",
            Status::Warning => "warning",
            Status::Skipped => "skipped",
        }
    }
}

#[derive(Serialize, Debug)]
pub(crate) struct Check {
    id: &'static str,
    status: Status,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<String>,
    /// The agent a model-routing check is about, when the policy has an entry for each agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<&'static str>,
}

impl Check {
    /// Messages can contain text from the repository, so control characters never reach the terminal.
    fn new(
        id: &'static str,
        status: Status,
        message: impl AsRef<str>,
        fix: Option<String>,
    ) -> Self {
        Self {
            id,
            status,
            message: printable(message.as_ref()),
            fix,
            agent: None,
        }
    }
    /// Says which agent this check is about.
    fn for_agent(mut self, agent: &'static str) -> Self {
        self.agent = Some(agent);
        self
    }
    fn pass(id: &'static str, message: impl AsRef<str>) -> Self {
        Self::new(id, Status::Pass, message, None)
    }
    fn missing(id: &'static str, message: impl AsRef<str>, fix: impl Into<String>) -> Self {
        Self::new(id, Status::Missing, message, Some(fix.into()))
    }
    fn fail(id: &'static str, message: impl AsRef<str>, fix: impl Into<String>) -> Self {
        Self::new(id, Status::Fail, message, Some(fix.into()))
    }
    fn warning(id: &'static str, message: impl AsRef<str>, fix: impl Into<String>) -> Self {
        Self::new(id, Status::Warning, message, Some(fix.into()))
    }
    fn skipped(id: &'static str, message: impl AsRef<str>) -> Self {
        Self::new(id, Status::Skipped, message, None)
    }
    fn passed(&self) -> bool {
        self.status == Status::Pass
    }
    /// Whether this check makes the factory not ready. Warnings and skipped checks never do.
    fn blocks(&self) -> bool {
        matches!(self.status, Status::Missing | Status::Fail)
    }
}

/// Replaces control characters, such as terminal escape sequences, with a visible placeholder.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

/// A remote URL for display: no credentials, query or fragment.
fn display_url(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            if path.is_empty() {
                format!("{scheme}://{host}")
            } else {
                format!("{scheme}://{host}/{path}")
            }
        }
        // scp-like `user@host:path` carries a user name, not a secret, but is shortened the same way.
        None => url
            .rsplit_once('@')
            .map_or(url, |(_, rest)| rest)
            .to_string(),
    }
}

#[derive(Serialize, Debug)]
pub(crate) struct AgentReport {
    id: &'static str,
    usable: bool,
    checks: Vec<Check>,
}

#[derive(Serialize, Debug)]
pub(crate) struct Report {
    pub(crate) ok: bool,
    project: Vec<Check>,
    agents: Vec<AgentReport>,
    /// The pull request host and its tools. These checks never change `ok`.
    provider: Vec<Check>,
    /// Present only when a model-routing policy exists.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    model_routing: Vec<Check>,
}

/// Everything the checks look at besides the project folder, so tests can supply their own.
pub(crate) struct Machine<'a> {
    pub(crate) home: PathBuf,
    pub(crate) path: OsString,
    pub(crate) var: &'a dyn Fn(&str) -> Option<OsString>,
    /// The `--config-dir` values: they replace the Claude config directory.
    pub(crate) config_dirs: Vec<String>,
    /// What an agent's program prints for `--version`, if it can be asked.
    pub(crate) version: &'a dyn Fn(&Path) -> Option<String>,
    /// What `rustup target list --installed` prints, if rustup can be asked.
    pub(crate) rustup_targets: &'a dyn Fn(&OsString, &Path) -> Option<String>,
}

/// Checks the project at `project` and the listed agents (all of them when `agent` is `None`).
pub(crate) fn diagnose(
    project: &Path,
    agent: Option<&str>,
    machine: &Machine,
) -> io::Result<Report> {
    let agent_ids: Vec<&'static str> = match agent {
        None => AGENTS.to_vec(),
        Some(name) => vec![*AGENTS.iter().find(|known| **known == name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown agent {name:?}; use codex, claude, or copilot"),
            )
        })?],
    };
    if !machine.config_dirs.is_empty() && agent.is_some_and(|name| name != "claude") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--config-dir applies to the claude agent; use --agent claude or leave --agent out",
        ));
    }
    // Resolved before any check runs, so a relative path fails the whole command.
    claude_config_dirs(&machine.config_dirs, &machine.home, machine.var)?;
    let git = git_checks(project);
    // Later checks read from the work tree root, so they work from a subfolder or a linked worktree.
    let root = git_output(project, &["rev-parse", "--show-toplevel"])
        .map_or_else(|| project.to_path_buf(), PathBuf::from);
    let mut project_checks = git;
    project_checks.push(setup_docs_check(&root));
    project_checks.push(verification_check(&root, &machine.path));
    // A warning never blocks, so this check stays out of `shared_ok`.
    let windows_lint = windows_lint_check(&root, machine);
    let agents: Vec<AgentReport> = agent_ids
        .into_iter()
        .map(|id| {
            let mut checks = vec![executable_check(id, &machine.path)];
            checks.extend(skills_checks(id, machine));
            AgentReport {
                id,
                usable: checks.iter().all(Check::passed),
                checks,
            }
        })
        .collect();
    let shared_ok = project_checks.iter().all(Check::passed);
    project_checks.extend(windows_lint);
    let agents_ok = match agent {
        Some(_) => agents.iter().all(|a| a.usable),
        None => agents.iter().any(|a| a.usable),
    };
    let model_routing = routing::checks(&root, machine, agent);
    let provider = provider::checks(project, machine);
    Ok(Report {
        ok: shared_ok && agents_ok && !model_routing.iter().any(Check::blocks),
        project: project_checks,
        agents,
        provider,
        model_routing,
    })
}

pub(crate) fn git_output(project: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
}

fn git_checks(project: &Path) -> Vec<Check> {
    match Command::new("git").arg("--version").output() {
        Ok(out) if out.status.success() => {}
        _ => {
            return vec![Check::fail(
                "git",
                "git is not available",
                "Install Git and make sure `git` is on PATH.",
            )]
        }
    }
    if git_output(project, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return vec![Check::fail(
            "git",
            "this folder is not inside a Git repository",
            "Run the command from your project, or create a repository with `git init`.",
        )];
    }
    let mut checks = vec![Check::pass("git", "inside a Git repository")];
    match git_output(project, &["remote", "get-url", "origin"]) {
        None => {
            checks.push(Check::fail(
                "origin",
                "there is no `origin` remote",
                "Add one with `git remote add origin <url>`.",
            ));
            return checks;
        }
        Some(url) => checks.push(Check::pass(
            "origin",
            format!("origin is {}", display_url(&url)),
        )),
    }
    checks.push(
        match git_output(project, &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"]) {
            Some(head) => {
                let branch = head.strip_prefix("origin/").unwrap_or(&head);
                Check::pass("base-branch", format!("origin's default branch is {branch}"))
            }
            None => Check::missing(
                "base-branch",
                "git does not know origin's default branch",
                "Run `git remote set-head origin --auto`, or `git remote set-head origin <branch>`.",
            ),
        },
    );
    checks
}

/// The first of `AGENTS.md` and `CLAUDE.md` that is a regular file.
fn conventions_file(root: &Path) -> Option<PathBuf> {
    ["AGENTS.md", "CLAUDE.md"]
        .iter()
        .map(|name| root.join(name))
        .find(|path| fs::metadata(path).is_ok_and(|meta| meta.is_file()))
}

/// Reads a project file. The project is not trusted: only a regular file is read, and never more than
/// `MAX_DOC_BYTES`, so a device, a pipe or a file that grows cannot make doctor wait or fill memory.
fn read_doc(path: &Path) -> io::Result<String> {
    use io::Read;
    // Looked at before it is opened, because opening a named pipe waits for a writer. `metadata` follows
    // symlinks. The opened file is checked again below in case it was swapped in between.
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    if metadata.len() > MAX_DOC_BYTES {
        return Err(io::Error::other("larger than 1 MiB"));
    }
    let mut text = String::new();
    file.take(MAX_DOC_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_DOC_BYTES {
        return Err(io::Error::other("larger than 1 MiB"));
    }
    Ok(text)
}

fn setup_docs_check(root: &Path) -> Check {
    const FIX: &str = "Run `/factory-setup` once in the project and commit its output.";
    let mut missing = Vec::new();
    if !root.join("docs/agents/issue-tracker.md").is_file() {
        missing.push("docs/agents/issue-tracker.md".to_string());
    }
    match conventions_file(root) {
        None => missing.push("AGENTS.md or CLAUDE.md".to_string()),
        Some(file) => match read_doc(&file) {
            Ok(text)
                if text
                    .lines()
                    .any(|line| line.trim_end() == "## Agent skills") => {}
            Ok(_) => missing.push(format!(
                "an \"## Agent skills\" section in {}",
                file.file_name().unwrap_or_default().to_string_lossy()
            )),
            Err(error) => {
                return Check::fail(
                    "setup-docs",
                    format!("cannot read {}: {error}", file.display()),
                    FIX,
                )
            }
        },
    }
    if missing.is_empty() {
        Check::pass(
            "setup-docs",
            "issue tracker docs and the Agent skills section are in place",
        )
    } else {
        Check::missing(
            "setup-docs",
            format!(
                "the factory setup is incomplete: {} (needed by /factory-full, and by the lighter workflows when a run reads or publishes a spec or tickets)",
                missing.join(", ")
            ),
            FIX,
        )
    }
}

/// The commands documented as bullets under `## Commands`, e.g. ``- Test: `cargo test` ``.
fn documented_commands(text: &str) -> Option<Vec<String>> {
    let mut lines = text
        .lines()
        .skip_while(|line| line.trim_end() != "## Commands");
    lines.next()?;
    let mut commands = Vec::new();
    for line in lines.take_while(|line| !line.starts_with("## ")) {
        let line = line.trim_start();
        if !(line.starts_with("- ") || line.starts_with("* ")) {
            continue;
        }
        let mut parts = line.split('`');
        parts.next();
        while let (Some(code), _) = (parts.next(), parts.next()) {
            if !code.trim().is_empty() {
                commands.push(code.trim().to_string());
            }
        }
    }
    Some(commands)
}

/// For a Rust project on a non-Windows host: whether a Windows target is installed, so verification can lint Windows-only code.
fn windows_lint_check(root: &Path, machine: &Machine) -> Option<Check> {
    if cfg!(windows) || !root.join("Cargo.toml").is_file() {
        return None;
    }
    let Some(targets) = (machine.rustup_targets)(&machine.path, &machine.home) else {
        return Some(Check::skipped(
            "windows-lint",
            "rustup is not available, so Windows targets were not checked",
        ));
    };
    Some(if targets.lines().any(|t| t.contains("-pc-windows-")) {
        Check::pass("windows-lint", "a Windows target is installed")
    } else {
        Check::warning(
            "windows-lint",
            "no Windows target is installed, so Windows-only code is not linted locally",
            "Run `rustup target add x86_64-pc-windows-gnu` so the project's verification can lint Windows-only code.",
        )
    })
}

fn verification_check(root: &Path, path: &OsString) -> Check {
    const FIX: &str = "Add a \"## Commands\" section to AGENTS.md or CLAUDE.md with the checks to run before merging, one per bullet, for example \"- Test: `cargo test`\".";
    let Some(file) = conventions_file(root) else {
        return Check::missing(
            "verification",
            "no AGENTS.md or CLAUDE.md to read the commands from",
            FIX,
        );
    };
    let text = match read_doc(&file) {
        Ok(text) => text,
        Err(error) => {
            return Check::fail(
                "verification",
                format!("cannot read {}: {error}", file.display()),
                FIX,
            )
        }
    };
    let commands = documented_commands(&text).unwrap_or_default();
    if commands.is_empty() {
        return Check::missing(
            "verification",
            "no verification commands under \"## Commands\"",
            FIX,
        );
    }
    // Only a plain program name or path is looked up; anything with shell syntax is left alone.
    let not_found: Vec<String> = commands
        .iter()
        .filter_map(|command| command.split_whitespace().next())
        .filter(|program| {
            program
                .chars()
                .all(|c| c.is_alphanumeric() || "._-/+".contains(c))
        })
        .filter(|program| {
            if program.contains('/') {
                !root.join(program).is_file()
            } else {
                find_program(program, path).is_none()
            }
        })
        .map(str::to_string)
        .collect();
    if not_found.is_empty() {
        Check::pass(
            "verification",
            format!("{} documented command(s), all tools found", commands.len()),
        )
    } else {
        Check::missing(
            "verification",
            format!(
                "documented commands need tools that were not found: {}",
                not_found.join(", ")
            ),
            "Install the missing tools, or correct the commands in the \"## Commands\" section.",
        )
    }
}

/// Finds `name` in the `PATH` folders. On Windows a name without a `PATHEXT` extension is tried with each
/// extension, and a name that already ends in one, such as `npm.cmd`, is tried as given first.
pub(crate) fn find_program(name: &str, path: &OsString) -> Option<PathBuf> {
    let extensions = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        windows_extensions(&pathext)
    } else {
        vec![String::new()]
    };
    find_with_extensions(name, path, &extensions)
}

/// What to run for `az`. Windows starts only `az.exe` for a bare name, but the Azure CLI installs `az.cmd`,
/// so the launcher found on `PATH` is used.
pub(crate) fn az_program() -> PathBuf {
    std::env::var_os("PATH").map_or_else(|| PathBuf::from("az"), |path| az_program_in(&path))
}

fn az_program_in(path: &OsString) -> PathBuf {
    find_program("az", path)
        .filter(|found| found.is_absolute())
        .unwrap_or_else(|| PathBuf::from("az"))
}

/// The extensions Windows runs, in `PATHEXT` order. A file without one, such as the Bash launcher
/// next to `az.cmd`, is not something Windows can run, so it is not a candidate.
fn windows_extensions(pathext: &str) -> Vec<String> {
    pathext
        .split(';')
        .filter(|ext| ext.starts_with('.'))
        .map(str::to_string)
        .collect()
}

/// The suffixes to append to `name`: the extensions, preceded by the empty suffix when `name` already
/// ends in one of them (compared case-insensitively).
fn candidate_suffixes(name: &str, extensions: &[String]) -> Vec<String> {
    let lower = name.to_lowercase();
    let has_extension = extensions
        .iter()
        .any(|ext| !ext.is_empty() && lower.ends_with(&ext.to_lowercase()));
    let mut suffixes = Vec::new();
    if has_extension {
        suffixes.push(String::new());
    }
    suffixes.extend(extensions.iter().cloned());
    suffixes
}

fn find_with_extensions(name: &str, path: &OsString, extensions: &[String]) -> Option<PathBuf> {
    let suffixes = candidate_suffixes(name, extensions);
    std::env::split_paths(path).find_map(|dir| {
        suffixes.iter().find_map(|suffix| {
            let candidate = dir.join(format!("{name}{suffix}"));
            is_executable(&candidate).then_some(candidate)
        })
    })
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn executable_check(agent: &'static str, path: &OsString) -> Check {
    match find_program(agent, path) {
        Some(found) => Check::pass(
            "executable",
            format!("`{agent}` found at {}", found.display()),
        ),
        None => Check::missing(
            "executable",
            format!("`{agent}` is not on PATH"),
            format!("Install the {agent} command line tool and make sure `{agent}` is on PATH."),
        ),
    }
}

/// One skills check per skills folder: each Claude config directory has its own.
fn skills_checks(agent: &'static str, machine: &Machine) -> Vec<Check> {
    let roots = if agent == "claude" {
        claude_config_dirs(&machine.config_dirs, &machine.home, machine.var)
            .map(|dirs| dirs.into_iter().map(|dir| dir.join("skills")).collect())
    } else {
        skills_root(agent, &machine.home, machine.var).map(|root| vec![root])
    };
    match roots {
        Ok(roots) => roots
            .iter()
            .map(|root| skills_check(agent, root, !machine.config_dirs.is_empty()))
            .collect(),
        Err(error) => vec![Check::fail(
            "skills",
            error.to_string(),
            "Use codex, claude, or copilot.",
        )],
    }
}

fn skills_check(agent: &'static str, root: &Path, explicit_dir: bool) -> Check {
    // A skill counts as installed only when every file `install` writes for it is there, so an
    // install from an older release that lacks newer files is reported too.
    let mut absent: Vec<&str> = bundled_skill_files()
        .filter(|file| !root.join(file).is_file())
        .filter_map(|file| file.split('/').next())
        .collect();
    absent.dedup();
    if absent.is_empty() {
        Check::pass(
            "skills",
            format!("all factory skills are installed in {}", root.display()),
        )
    } else {
        Check::missing(
            "skills",
            format!(
                "skills not installed in {}: {}",
                root.display(),
                absent.join(", ")
            ),
            if explicit_dir && agent == "claude" {
                let dir = root.parent().unwrap_or(root);
                format!(
                    "Run `factory install --agent claude --config-dir {}`.",
                    dir.display()
                )
            } else {
                format!("Run `factory install --agent {agent}`.")
            },
        )
    }
}

/// The human-readable report.
pub(crate) fn render(report: &Report) -> String {
    fn line(check: &Check, indent: &str) -> String {
        let tag = check
            .agent
            .map_or(String::new(), |agent| format!(" [{agent}]"));
        let mut text = format!(
            "{indent}{:<8} {}{tag}: {}\n",
            check.status.as_str(),
            check.id,
            check.message
        );
        if let Some(fix) = &check.fix {
            text.push_str(&format!("{indent}         Fix: {fix}\n"));
        }
        text
    }
    let mut out = String::from("Project\n");
    for check in &report.project {
        out.push_str(&line(check, "  "));
    }
    out.push_str("\nAgents\n");
    for agent in &report.agents {
        out.push_str(&format!(
            "  {}: {}\n",
            agent.id,
            if agent.usable { "usable" } else { "not usable" }
        ));
        for check in &agent.checks {
            out.push_str(&line(check, "    "));
        }
    }
    out.push_str("\nProvider\n");
    for check in &report.provider {
        out.push_str(&line(check, "  "));
    }
    if !report.model_routing.is_empty() {
        out.push_str("\nModel routing\n");
        for check in &report.model_routing {
            out.push_str(&line(check, "  "));
        }
    }
    out.push_str(if report.ok {
        "\nReady: the factory can run here.\n"
    } else {
        "\nNot ready: fix the items above and run `factory doctor` again.\n"
    });
    out
}

/// Asks an agent's program for its version. This is the only command doctor runs, and it calls no model.
fn agent_version(program: &Path) -> Option<String> {
    // A relative path, such as one found through an empty `PATH` entry, would run a file from the project.
    if !program.is_absolute() {
        return None;
    }
    let output = Command::new(program).arg("--version").output().ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .chars()
            .take(200)
            .collect()
    })
}

/// Asks rustup which targets are installed, looking for it on `path`. Read-only.
/// It runs outside the project, in `home`, with auto-install off, so a `rust-toolchain.toml` cannot make it download.
fn rustup_targets(path: &OsString, home: &Path) -> Option<String> {
    let program = find_program("rustup", path)?;
    if !program.is_absolute() {
        return None;
    }
    let output = Command::new(program)
        .args(["target", "list", "--installed"])
        .current_dir(home)
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Runs the checks for the current folder and prints the report. Returns whether the factory is ready.
pub(crate) fn run(args: Args) -> Result<bool, Box<dyn std::error::Error>> {
    let machine = Machine {
        home: crate::install::home_dir()?,
        path: std::env::var_os("PATH").unwrap_or_default(),
        var: &|name| std::env::var_os(name),
        config_dirs: args.config_dirs.iter().map(|dir| dir.to_string()).collect(),
        version: &agent_version,
        rustup_targets: &rustup_targets,
    };
    let Args {
        agent,
        json,
        config_dirs: _,
        create_model_routing: create,
    } = args;
    if let Some(create_for) = create {
        // With `--json` the note goes to stderr: stdout stays JSON.
        let note = create_model_routing(&create_for, &machine)?;
        if json {
            eprintln!("{note}");
        } else {
            println!("{note}\n");
        }
    }
    let report = diagnose(&std::env::current_dir()?, agent, &machine)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render(&report));
    }
    Ok(report.ok)
}

/// What `factory doctor` was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Args<'a> {
    pub(crate) agent: Option<&'a str>,
    pub(crate) json: bool,
    /// The `--config-dir` values, in the order given.
    pub(crate) config_dirs: Vec<&'a str>,
    /// Write a starter model-routing policy first, when none exists, with an entry for each of these agents.
    pub(crate) create_model_routing: Option<Vec<RoutingAgent>>,
}

/// Writes a starter model-routing policy for `agent` unless a file is already there, and says what it did.
/// An old-format file is replaced.
/// This is the one thing doctor writes, and only when asked.
fn create_model_routing(agents: &[RoutingAgent], machine: &Machine) -> Result<String, String> {
    let path = policy_path_in(|name| (machine.var)(name)).ok_or(
        if cfg!(windows) {
            "cannot work out the configuration folder for the model-routing policy; set APPDATA"
        } else {
            "cannot work out the configuration folder for the model-routing policy; set XDG_CONFIG_HOME or HOME"
        },
    )?;
    let shown = printable(&path.display().to_string());
    match install_starter(&path, agents) {
        Ok(done @ (Starter::Created | Starter::Replaced(_))) => {
            let names: Vec<_> = agents.iter().map(|agent| agent.name()).collect();
            let named = match names.split_last() {
                Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
                _ => names.join(""),
            };
            let about = format!(
                "It is a starter policy for {named} that routes every stage (define, build, check, finish) of each."
            );
            Ok(match done {
                Starter::Replaced(off) => Starter::replaced_message(&shown, off, Some(&about)),
                _ => Starter::created_message(&shown, Some(&about)),
            })
        }
        // A link that points nowhere is "there" for creating but not for reading, so say which it is.
        Ok(Starter::Left) if fs::metadata(&path).is_err() => Ok(format!(
            "{shown} is a link that points nowhere, so no policy is read from it. It was left unchanged."
        )),
        Ok(Starter::Left) => Ok(format!("{shown} already exists and was left unchanged.")),
        Err(error) => Err(format!(
            "cannot create {shown}: {error}. Check that the folder can be written, or create the file yourself with the example in the README (Model routing)."
        )),
    }
}

/// Reads `[--agent <name>] [--config-dir <path>]... [--json] [--create-model-routing]` in any order.
pub(crate) fn parse_args<'a>(args: &[&'a str]) -> Result<Args<'a>, String> {
    let (mut agent, mut json, mut create) = (None, false, false);
    let mut config_dirs = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--json" => json = true,
            "--create-model-routing" => create = true,
            "--config-dir" => match args.next() {
                Some(dir) => config_dirs.push(*dir),
                None => return Err("--config-dir needs a path.".into()),
            },
            "--agent" => match args.next() {
                Some(name) if AGENTS.contains(name) && agent.is_none() => agent = Some(*name),
                Some(name) if AGENTS.contains(name) => {
                    return Err("--agent can be given only once.".into())
                }
                Some(name) => {
                    return Err(format!(
                        "Unknown agent {name:?}; use codex, claude, or copilot."
                    ))
                }
                None => return Err("--agent needs codex, claude, or copilot.".into()),
            },
            other => return Err(format!("Unknown option {other:?}.")),
        }
    }
    let create_model_routing = match (create, agent) {
        (false, _) => None,
        (true, Some("claude")) => Some(vec![RoutingAgent::Claude]),
        (true, Some("codex")) => Some(vec![RoutingAgent::Codex]),
        (true, Some("copilot")) => Some(vec![RoutingAgent::Copilot]),
        // Without an agent the starter has an entry for each, which is what one file for all is for.
        (true, None) => Some(vec![
            RoutingAgent::Claude,
            RoutingAgent::Codex,
            RoutingAgent::Copilot,
        ]),
        (true, Some(other)) => return Err(format!("Unknown agent {other:?}.")),
    };
    Ok(Args {
        agent,
        json,
        config_dirs,
        create_model_routing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::bundled_skill_names;
    use std::collections::HashMap;
    use tempfile::TempDir;

    /// A project, a home folder and a `PATH` folder, all temporary.
    struct World {
        project: TempDir,
        home: TempDir,
        bin: TempDir,
        vars: HashMap<String, OsString>,
        /// What each agent's program prints for `--version`.
        versions: HashMap<String, String>,
        /// What `rustup target list --installed` prints; `None` when rustup is not available.
        rustup_targets: Option<String>,
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn program(bin: &Path, name: &str) {
        let file = bin.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        });
        fs::write(&file, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn install_skills(root: &Path) {
        for file in bundled_skill_files() {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "file").unwrap();
        }
    }

    const AGENTS_MD: &str = "# Project\n\n## Commands\n\n- Test: `cargo test`\n- Lint: `cargo clippy -- -D warnings`\n\n## Agent skills\n\nSee docs.\n";

    impl World {
        /// A ready project (repository, origin, base branch, setup docs, commands) with `cargo` on PATH and nothing else.
        fn ready() -> Self {
            let world = Self {
                project: tempfile::tempdir().unwrap(),
                home: tempfile::tempdir().unwrap(),
                bin: tempfile::tempdir().unwrap(),
                vars: HashMap::new(),
                versions: HashMap::new(),
                rustup_targets: None,
            };
            let dir = world.project.path();
            git(dir, &["init", "-q"]);
            git(
                dir,
                &["remote", "add", "origin", "https://example.com/repo.git"],
            );
            git(
                dir,
                &[
                    "symbolic-ref",
                    "refs/remotes/origin/HEAD",
                    "refs/remotes/origin/develop",
                ],
            );
            fs::create_dir_all(dir.join("docs/agents")).unwrap();
            fs::write(dir.join("docs/agents/issue-tracker.md"), "tracker").unwrap();
            fs::write(dir.join("AGENTS.md"), AGENTS_MD).unwrap();
            program(world.bin.path(), "cargo");
            world
        }

        /// Makes `agent` usable: its command on PATH and its skills in the default folder.
        fn with_agent(self, agent: &str) -> Self {
            program(self.bin.path(), agent);
            let root =
                skills_root(agent, self.home.path(), |name| self.vars.get(name).cloned()).unwrap();
            install_skills(&root);
            self
        }

        fn report(&self, agent: Option<&str>) -> Report {
            self.report_in(agent, &[]).unwrap()
        }

        fn report_in(&self, agent: Option<&str>, config_dirs: &[&str]) -> io::Result<Report> {
            let machine = Machine {
                home: self.home.path().to_path_buf(),
                path: self.bin.path().into(),
                var: &|name| self.vars.get(name).cloned(),
                config_dirs: config_dirs.iter().map(|dir| dir.to_string()).collect(),
                version: &|program| {
                    let name = program.file_stem()?.to_str()?;
                    self.versions.get(name).cloned()
                },
                rustup_targets: &|_, _| self.rustup_targets.clone(),
            };
            diagnose(self.project.path(), agent, &machine)
        }
    }

    fn project_check<'a>(report: &'a Report, id: &str) -> &'a Check {
        report
            .project
            .iter()
            .find(|check| check.id == id)
            .unwrap_or_else(|| panic!("no {id} check"))
    }

    fn agent<'a>(report: &'a Report, id: &str) -> &'a AgentReport {
        report.agents.iter().find(|a| a.id == id).unwrap()
    }

    #[test]
    fn windows_lint_is_a_non_blocking_warning_without_a_windows_target() {
        if cfg!(windows) {
            return;
        }
        let mut world = World::ready().with_agent("claude");
        fs::write(world.project.path().join("Cargo.toml"), "").unwrap();
        world.rustup_targets = Some("x86_64-unknown-linux-gnu\n".into());
        let report = world.report(None);
        let check = project_check(&report, "windows-lint");
        assert_eq!(check.status, Status::Warning);
        assert!(check
            .fix
            .as_deref()
            .unwrap()
            .contains("rustup target add x86_64-pc-windows-gnu"));
        assert!(report.ok, "{}", render(&report));
    }

    #[test]
    fn windows_lint_passes_with_a_windows_target_and_skips_without_rustup() {
        if cfg!(windows) {
            return;
        }
        let mut world = World::ready().with_agent("claude");
        fs::write(world.project.path().join("Cargo.toml"), "").unwrap();
        world.rustup_targets = Some("x86_64-pc-windows-gnu\nx86_64-unknown-linux-gnu\n".into());
        assert!(project_check(&world.report(None), "windows-lint").passed());
        world.rustup_targets = None;
        let report = world.report(None);
        assert_eq!(
            project_check(&report, "windows-lint").status,
            Status::Skipped
        );
        assert!(report.ok);
    }

    #[test]
    fn windows_lint_is_left_out_of_a_project_without_cargo_toml() {
        let mut world = World::ready().with_agent("claude");
        world.rustup_targets = Some(String::new());
        let report = world.report(None);
        assert!(report
            .project
            .iter()
            .all(|check| check.id != "windows-lint"));
    }

    #[test]
    fn a_ready_project_with_one_usable_agent_passes() {
        let report = World::ready().with_agent("claude").report(None);
        assert!(report.ok, "{}", render(&report));
        assert!(report.project.iter().all(Check::passed));
        assert!(agent(&report, "claude").usable);
        assert!(!agent(&report, "codex").usable);
        assert!(project_check(&report, "base-branch")
            .message
            .contains("develop"));
    }

    #[test]
    fn without_any_usable_agent_the_report_is_not_ok_and_says_how_to_fix_each() {
        let report = World::ready().report(None);
        assert!(!report.ok);
        for a in &report.agents {
            assert!(!a.usable);
            let fixes: Vec<_> = a.checks.iter().filter_map(|c| c.fix.as_deref()).collect();
            assert!(
                fixes
                    .iter()
                    .any(|fix| fix.contains(&format!("factory install --agent {}", a.id))),
                "{fixes:?}"
            );
        }
    }

    #[test]
    fn with_an_agent_option_only_that_agent_is_checked_and_required() {
        let world = World::ready().with_agent("claude");
        let claude = world.report(Some("claude"));
        assert!(claude.ok);
        assert_eq!(claude.agents.len(), 1);
        // Claude is usable, but the selected agent is not.
        let codex = world.report(Some("codex"));
        assert!(!codex.ok);
        assert_eq!(codex.agents.len(), 1);
        assert_eq!(codex.agents[0].id, "codex");
    }

    #[test]
    fn an_agent_needs_both_its_command_and_every_bundled_skill() {
        let world = World::ready().with_agent("claude");
        let skill = skills_root("claude", world.home.path(), |_| None)
            .unwrap()
            .join(bundled_skill_names()[0]);
        fs::remove_dir_all(skill).unwrap();
        let report = world.report(Some("claude"));
        assert!(!report.ok);
        let skills = &agent(&report, "claude").checks[1];
        assert_eq!(skills.status, Status::Missing);
        assert!(skills.message.contains(bundled_skill_names()[0]));

        // A skill that lost one of its other files is incomplete too.
        let world = World::ready().with_agent("claude");
        let root = skills_root("claude", world.home.path(), |_| None).unwrap();
        let extra = bundled_skill_files()
            .find(|file| !file.ends_with("/SKILL.md"))
            .expect("a skill ships more than SKILL.md");
        fs::remove_file(root.join(extra)).unwrap();
        let report = world.report(Some("claude"));
        assert_eq!(agent(&report, "claude").checks[1].status, Status::Missing);
        assert!(agent(&report, "claude").checks[1]
            .message
            .contains(extra.split('/').next().unwrap()));

        let world = World::ready();
        install_skills(&skills_root("codex", world.home.path(), |_| None).unwrap());
        let report = world.report(Some("codex"));
        assert!(!report.ok);
        assert_eq!(
            agent(&report, "codex").checks[0].status,
            Status::Missing,
            "no codex command"
        );
    }

    #[test]
    fn skills_are_looked_up_where_install_would_put_them() {
        let mut world = World::ready();
        let custom = tempfile::tempdir().unwrap();
        world
            .vars
            .insert("CLAUDE_CONFIG_DIR".into(), custom.path().into());
        world
            .vars
            .insert("CODEX_HOME".into(), custom.path().join("codex").into());
        world
            .vars
            .insert("COPILOT_HOME".into(), custom.path().join("copilot").into());
        let world = world
            .with_agent("claude")
            .with_agent("codex")
            .with_agent("copilot");
        assert!(custom
            .path()
            .join("skills")
            .join(bundled_skill_names()[0])
            .is_dir());
        assert!(custom.path().join("codex/skills").is_dir());
        assert!(custom.path().join("copilot/skills").is_dir());
        assert!(!world.home.path().join(".claude").exists());
        assert!(!world.home.path().join(".copilot").exists());
        let report = world.report(None);
        assert!(agent(&report, "claude").usable);
        assert!(agent(&report, "codex").usable);
        assert!(agent(&report, "copilot").usable);
    }

    #[test]
    fn project_problems_are_named_with_a_fix_and_make_the_report_not_ok() {
        // Nothing but an empty folder.
        let world = World::ready().with_agent("claude");
        let empty = tempfile::tempdir().unwrap();
        let machine = Machine {
            home: world.home.path().into(),
            path: world.bin.path().into(),
            var: &|_| None,
            config_dirs: Vec::new(),
            version: &|_| None,
            rustup_targets: &|_, _| None,
        };
        let report = diagnose(empty.path(), Some("claude"), &machine).unwrap();
        assert!(!report.ok);
        assert_eq!(project_check(&report, "git").status, Status::Fail);

        let no_origin = World::ready().with_agent("claude");
        git(no_origin.project.path(), &["remote", "remove", "origin"]);
        let report = no_origin.report(None);
        assert!(!report.ok);
        assert_eq!(project_check(&report, "origin").status, Status::Fail);
        assert!(project_check(&report, "origin")
            .fix
            .as_ref()
            .unwrap()
            .contains("git remote add origin"));

        let no_base = World::ready().with_agent("claude");
        git(
            no_base.project.path(),
            &["symbolic-ref", "--delete", "refs/remotes/origin/HEAD"],
        );
        let report = no_base.report(None);
        assert!(!report.ok);
        assert_eq!(
            project_check(&report, "base-branch").status,
            Status::Missing
        );
        assert!(project_check(&report, "base-branch")
            .fix
            .as_ref()
            .unwrap()
            .contains("set-head"));
    }

    #[test]
    fn setup_docs_and_documented_commands_are_checked_without_running_anything() {
        let world = World::ready().with_agent("claude");
        let dir = world.project.path();
        fs::remove_file(dir.join("docs/agents/issue-tracker.md")).unwrap();
        fs::write(
            dir.join("AGENTS.md"),
            "# Project\n\n## Commands\n\n- Test: `cargo test`\n",
        )
        .unwrap();
        let report = world.report(None);
        let docs = project_check(&report, "setup-docs");
        assert_eq!(docs.status, Status::Missing);
        assert!(docs.message.contains("issue-tracker.md") && docs.message.contains("Agent skills"));
        assert!(docs.fix.as_ref().unwrap().contains("/factory-setup"));

        // A command whose tool is absent is reported, and a script path is looked up in the project.
        fs::write(
            dir.join("AGENTS.md"),
            "## Commands\n\n- Verify: `scripts/verify.sh`\n- Lint: `nonexistent-tool --check`\n\n## Agent skills\n",
        )
        .unwrap();
        let report = world.report(None);
        let verification = project_check(&report, "verification");
        assert_eq!(verification.status, Status::Missing);
        assert!(verification.message.contains("nonexistent-tool"));
        assert!(verification.message.contains("scripts/verify.sh"));
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(dir.join("scripts/verify.sh"), "#!/bin/sh\ntouch RAN\n").unwrap();
        fs::write(
            dir.join("AGENTS.md"),
            "## Commands\n\n- Verify: `scripts/verify.sh`\n\n## Agent skills\n",
        )
        .unwrap();
        let report = world.report(None);
        assert!(project_check(&report, "verification").passed());
        assert!(
            !dir.join("RAN").exists(),
            "doctor must not run the project's commands"
        );

        fs::write(
            dir.join("AGENTS.md"),
            "# Nothing about commands\n\n## Agent skills\n",
        )
        .unwrap();
        let report = world.report(None);
        let verification = project_check(&report, "verification");
        assert_eq!(verification.status, Status::Missing);
        assert!(verification.fix.as_ref().unwrap().contains("## Commands"));
    }

    #[test]
    fn the_origin_is_shown_without_credentials_or_control_characters() {
        let world = World::ready().with_agent("claude");
        git(
            world.project.path(),
            &[
                "remote",
                "set-url",
                "origin",
                "https://user:secret-token@example.com/me/repo.git?x=1#frag",
            ],
        );
        let report = world.report(None);
        let origin = &project_check(&report, "origin").message;
        assert_eq!(origin, "origin is https://example.com/me/repo.git");
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("secret-token"));

        assert_eq!(
            display_url("git@github.com:me/repo.git"),
            "github.com:me/repo.git"
        );
        assert_eq!(display_url("https://example.com"), "https://example.com");
        assert_eq!(printable("a\u{1b}[2Jb\nc"), "a\u{FFFD}[2Jb\u{FFFD}c");
        let check = Check::missing("x", "bad \u{1b}]0;title\u{7}", "fix");
        assert!(!check.message.chars().any(char::is_control));
    }

    #[test]
    fn claude_md_can_hold_the_conventions() {
        let world = World::ready().with_agent("claude");
        let dir = world.project.path();
        fs::rename(dir.join("AGENTS.md"), dir.join("CLAUDE.md")).unwrap();
        assert!(world.report(None).ok);
    }

    #[test]
    fn doctor_changes_nothing() {
        fn snapshot(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut entries: Vec<_> = fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    snapshot(&path, out);
                } else {
                    out.push((path.clone(), fs::read(&path).unwrap()));
                }
            }
        }
        let world = World::ready().with_agent("claude");
        let take = || {
            let mut all = Vec::new();
            for dir in [world.project.path(), world.home.path(), world.bin.path()] {
                snapshot(dir, &mut all);
            }
            all
        };
        let before = take();
        world.report(None);
        assert_eq!(before, take());
    }

    #[test]
    fn the_json_report_has_lowercase_statuses_and_fixes_only_where_needed() {
        let report = World::ready().with_agent("claude").report(None);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["project"][0]["id"], "git");
        assert_eq!(json["project"][0]["status"], "pass");
        assert!(json["project"][0].get("fix").is_none());
        let codex = json["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "codex")
            .unwrap();
        assert_eq!(codex["usable"], false);
        assert_eq!(codex["checks"][0]["status"], "missing");
        assert!(codex["checks"][0]["fix"]
            .as_str()
            .unwrap()
            .contains("codex"));
    }

    #[test]
    fn each_claude_config_dir_gets_its_own_skills_check() {
        let world = World::ready();
        program(world.bin.path(), "claude");
        let home = world.home.path();
        let (work, bare) = (home.join("work"), home.join("bare"));
        install_skills(&work.join("skills"));
        let dirs = [work.display().to_string(), bare.display().to_string()];
        let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
        let report = world.report_in(Some("claude"), &dirs).unwrap();
        let claude = agent(&report, "claude");
        let skills: Vec<&Check> = claude.checks.iter().filter(|c| c.id == "skills").collect();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].status, Status::Pass);
        assert!(skills[0].message.contains(&work.display().to_string()));
        assert_eq!(skills[1].status, Status::Missing);
        assert!(skills[1].message.contains(&bare.display().to_string()));
        assert!(skills[1]
            .fix
            .as_deref()
            .unwrap()
            .contains(&format!("--config-dir {}", bare.display())));
        assert!(!claude.usable && !report.ok);
        // The default folder is not looked at once a directory is named.
        assert!(!home.join(".claude").exists());
        // Without the flag the report is unchanged: one check for the default folder.
        let report = world.report(Some("claude"));
        assert_eq!(
            agent(&report, "claude")
                .checks
                .iter()
                .filter(|c| c.id == "skills")
                .count(),
            1
        );
    }

    #[test]
    fn config_dirs_must_be_absolute_and_need_the_claude_agent() {
        let world = World::ready().with_agent("claude");
        let relative = world.report_in(Some("claude"), &["rel"]).unwrap_err();
        assert!(relative.to_string().contains("absolute"));
        let x = crate::install::test_abs("x").display().to_string();
        let codex = world.report_in(Some("codex"), &[&x]).unwrap_err();
        assert!(codex.to_string().contains("claude"));
        assert!(world.report_in(None, &[&x]).is_ok());
        assert_eq!(
            parse_args(&["--config-dir", "/a", "--json", "--config-dir", "~/b"])
                .unwrap()
                .config_dirs,
            ["/a", "~/b"]
        );
        assert!(parse_args(&["--config-dir"]).is_err());
    }

    #[test]
    fn the_text_report_shows_status_and_fix_per_line() {
        let text = render(&World::ready().with_agent("claude").report(None));
        assert!(text.contains("pass     git:"));
        assert!(text.contains("missing  executable: `codex` is not on PATH"));
        assert!(text.contains("Fix: Run `factory install --agent codex`."));
        assert!(text.contains("claude: usable") && text.contains("codex: not usable"));
        assert!(text.contains("Ready"));
    }

    #[test]
    fn arguments_accept_an_agent_and_json_in_any_order() {
        let args = |agent, json, create_model_routing| {
            Ok(Args {
                agent,
                json,
                config_dirs: Vec::new(),
                create_model_routing,
            })
        };
        assert_eq!(parse_args(&[]), args(None, false, None));
        assert_eq!(
            parse_args(&["--json", "--agent", "copilot"]),
            args(Some("copilot"), true, None)
        );
        assert_eq!(
            parse_args(&["--agent", "codex"]),
            args(Some("codex"), false, None)
        );
        assert_eq!(
            parse_args(&["--create-model-routing", "--json", "--agent", "claude"]),
            args(Some("claude"), true, Some(vec![RoutingAgent::Claude]))
        );
        assert_eq!(
            parse_args(&["--agent", "codex", "--create-model-routing"]),
            args(Some("codex"), false, Some(vec![RoutingAgent::Codex]))
        );
        for bad in [
            &["--agent"][..],
            &["--agent", "gemini"],
            &["--agent", "codex", "--agent", "claude"],
            &["--fix"],
            &["claude"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn creating_the_policy_covers_every_agent_unless_one_is_named() {
        let error = |args: &[&str]| parse_args(args).unwrap_err();
        let both = parse_args(&["--create-model-routing"]).unwrap();
        assert_eq!(
            both.create_model_routing,
            Some(vec![
                RoutingAgent::Claude,
                RoutingAgent::Codex,
                RoutingAgent::Copilot
            ])
        );
        let copilot = parse_args(&["--create-model-routing", "--agent", "copilot"]).unwrap();
        assert_eq!(
            copilot.create_model_routing,
            Some(vec![RoutingAgent::Copilot])
        );
        assert!(error(&["--create-model-routing", "--agent", "gemini"]).contains("Unknown agent"));
    }

    impl World {
        /// What `--create-model-routing` does on this fake machine.
        fn create_model_routing(&self, agents: &[RoutingAgent]) -> Result<String, String> {
            let machine = Machine {
                home: self.home.path().to_path_buf(),
                path: self.bin.path().into(),
                var: &|name| self.vars.get(name).cloned(),
                config_dirs: Vec::new(),
                version: &|_| None,
                rustup_targets: &|_, _| None,
            };
            create_model_routing(agents, &machine)
        }
    }

    #[test]
    fn doctor_creates_a_starter_policy_for_the_agent_and_then_checks_it() {
        let world = World::ready().claude_ready().with_config_folder();
        let message = world.create_model_routing(&[RoutingAgent::Claude]).unwrap();
        // Built part by part, so the separators are the ones the message shows on every system.
        let path = world
            .home
            .path()
            .join("config")
            .join("tds-factory")
            .join("model-routing.json");
        assert!(path.is_file(), "{message}");
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(
            message.contains("check") && message.contains("enabled"),
            "{message}"
        );
        // The next report already sees the file, and it passes every check.
        let report = world.report(None);
        assert!(report.ok, "{}", render(&report));
        assert!(
            report.model_routing.iter().all(Check::passed),
            "{}",
            render(&report)
        );
        // Created for Codex, the policy names Codex.
        let read = |world: &World| {
            fs::read_to_string(
                world
                    .home
                    .path()
                    .join("config/tds-factory/model-routing.json"),
            )
            .unwrap()
        };
        let text = read(&world);
        assert!(
            text.contains("\"claude\"") && !text.contains("\"codex\""),
            "{text}"
        );
        // For Codex only, and for both agents in one file.
        let codex = World::ready().with_config_folder();
        codex.create_model_routing(&[RoutingAgent::Codex]).unwrap();
        let text = read(&codex);
        assert!(
            text.contains("\"codex\"") && !text.contains("\"claude\""),
            "{text}"
        );
        let both = World::ready().with_config_folder();
        let message = both
            .create_model_routing(&[RoutingAgent::Claude, RoutingAgent::Codex])
            .unwrap();
        assert!(message.contains("claude and codex"), "{message}");
        let text = read(&both);
        assert!(
            text.contains("\"claude\"") && text.contains("\"codex\""),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_points_nowhere_is_left_alone_and_described_as_such() {
        let world = World::ready().with_config_folder();
        let folder = world.home.path().join("config/tds-factory");
        fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(
            folder.join("missing.json"),
            folder.join("model-routing.json"),
        )
        .unwrap();
        let message = world.create_model_routing(&[RoutingAgent::Claude]).unwrap();
        assert!(
            message.contains("points nowhere") && message.contains("unchanged"),
            "{message}"
        );
        assert!(!folder.join("missing.json").exists());
    }

    #[test]
    fn doctor_says_how_to_switch_on_a_starter_that_carries_a_switch_off() {
        let world = World::ready()
            .claude_ready()
            .with_policy(r#"{"enabled":false,"agent":"claude"}"#);
        let message = world.create_model_routing(&[RoutingAgent::Claude]).unwrap();
        assert!(message.contains("switched off"), "{message}");
        assert!(message.contains("to switch it on"), "{message}");
        assert!(!message.contains("to switch it off"), "{message}");

        let world = World::ready()
            .claude_ready()
            .with_policy(r#"{"agents":{"codex":{"enabled":false,"roles":{}}}}"#);
        let message = world
            .create_model_routing(&[RoutingAgent::Claude, RoutingAgent::Codex])
            .unwrap();
        assert!(
            message.contains("entry for codex is switched off")
                && message.contains("to switch it on"),
            "{message}"
        );
    }

    #[test]
    fn doctor_leaves_an_existing_policy_alone_and_says_so() {
        let world = World::ready().claude_ready().with_policy(CLAUDE_POLICY);
        let path = world
            .home
            .path()
            .join("config/tds-factory/model-routing.json");
        let message = world.create_model_routing(&[RoutingAgent::Claude]).unwrap();
        assert!(
            message.contains("already exists") && message.contains("unchanged"),
            "{message}"
        );
        assert_eq!(fs::read_to_string(path).unwrap(), CLAUDE_POLICY);
    }

    #[test]
    fn doctor_refuses_to_create_a_policy_it_cannot_place_or_that_is_not_supported() {
        // No configuration folder can be worked out.
        let world = World::ready();
        let error = world
            .create_model_routing(&[RoutingAgent::Claude])
            .unwrap_err();
        assert!(error.contains("configuration folder"), "{error}");
        // The folder is in the way: a file where the folder should be.
        let blocked = World::ready().with_config_folder();
        fs::write(blocked.home.path().join("config"), "a file").unwrap();
        let error = blocked
            .create_model_routing(&[RoutingAgent::Claude])
            .unwrap_err();
        assert!(
            error.contains("cannot create") && error.contains("create the file yourself"),
            "{error}"
        );
    }

    // --- one policy file for several agents ---------------------------------------------------------

    /// Claude and Codex in one file, with different models.
    const BOTH_POLICY: &str = r#"{"agents":{
        "claude":{"stages":{"check":{"model":"opus","effort":"high"}}},
        "codex":{"stages":{"check":{"model":"gpt-6-astra","effort":"high"}}}}}"#;

    fn tagged<'a>(report: &'a Report, agent: &str) -> Vec<&'a Check> {
        report
            .model_routing
            .iter()
            .filter(|c| c.agent == Some(agent))
            .collect()
    }

    #[test]
    fn checking_all_agents_reports_each_configured_entry_tagged_with_its_agent() {
        let world = World::ready()
            .claude_ready()
            .with_agent("codex")
            .with_version("codex", "codex-cli 0.166.0")
            .with_policy(BOTH_POLICY);
        let report = world.report(None);
        assert!(report.ok, "{}", render(&report));
        assert_eq!(report.model_routing[0].id, "policy");
        assert!(report.model_routing[0]
            .message
            .contains("configures claude, codex"));
        assert_eq!(report.model_routing[0].agent, None);
        let claude: Vec<_> = tagged(&report, "claude").iter().map(|c| c.id).collect();
        assert_eq!(claude, ["host", "model", "effort", "overrides"]);
        // Codex has its own checks, which skip what the factory cannot judge.
        let codex = tagged(&report, "codex");
        assert_eq!(codex.len(), 3);
        assert!(codex
            .iter()
            .any(|c| c.id == "model" && c.status == Status::Skipped));
        let text = render(&report);
        assert!(text.contains("pass     host [claude]:"), "{text}");
        assert!(text.contains("skipped  model [codex]:"), "{text}");
        let json = serde_json::to_value(&report).unwrap();
        assert!(json["model_routing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["agent"] == "codex"));
    }

    #[test]
    fn checking_one_agent_checks_only_its_entry() {
        let world = World::ready()
            .claude_ready()
            .with_agent("codex")
            .with_policy(BOTH_POLICY);
        let claude = world.report(Some("claude"));
        assert!(!tagged(&claude, "claude").is_empty() && tagged(&claude, "codex").is_empty());
        let codex = world.report(Some("codex"));
        assert!(!tagged(&codex, "codex").is_empty() && tagged(&codex, "claude").is_empty());
        // BOTH_POLICY has no Copilot entry, so Copilot is skipped with the reason.
        let copilot = world.report(Some("copilot"));
        assert!(tagged(&copilot, "claude").is_empty() && tagged(&copilot, "codex").is_empty());
        assert!(tagged(&copilot, "copilot")
            .iter()
            .any(|c| c.status == Status::Skipped && c.message.contains("no entry for copilot")));
    }

    #[test]
    fn an_agent_without_an_entry_is_skipped_when_asked_for_and_left_out_otherwise() {
        let only_claude =
            r#"{"agents":{"claude":{"stages":{"check":{"model":"opus","effort":"high"}}}}}"#;
        let world = World::ready()
            .claude_ready()
            .with_agent("codex")
            .with_policy(only_claude);
        let codex = world.report(Some("codex"));
        let skipped = tagged(&codex, "codex");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].status, Status::Skipped);
        assert!(
            skipped[0].message.contains("no entry for codex"),
            "{}",
            skipped[0].message
        );
        assert!(codex.ok);
        assert!(tagged(&world.report(None), "codex").is_empty());
    }

    #[test]
    fn a_switched_off_entry_is_skipped_and_an_invalid_one_fails_the_report() {
        let off = BOTH_POLICY.replace(
            "\"codex\":{\"stages\"",
            "\"codex\":{\"enabled\":false,\"stages\"",
        );
        let world = World::ready()
            .claude_ready()
            .with_agent("codex")
            .with_policy(&off);
        for ask in [Some("codex"), None] {
            let report = world.report(ask);
            let codex = tagged(&report, "codex");
            assert_eq!(codex.len(), 1, "{ask:?}");
            assert_eq!(codex[0].status, Status::Skipped);
            assert!(codex[0].message.contains("switched off"));
            assert!(report.ok);
        }
        let invalid = BOTH_POLICY.replace("gpt-6-astra", "gpt 6");
        let world = World::ready()
            .claude_ready()
            .with_agent("codex")
            .with_policy(&invalid);
        for ask in [Some("claude"), Some("codex"), None] {
            let report = world.report(ask);
            assert!(!report.ok, "{ask:?}");
            let check = routing_check(&report, "policy");
            assert_eq!(check.status, Status::Fail);
            assert!(check.message.contains("agents.codex"), "{}", check.message);
        }
    }

    #[test]
    fn a_policy_for_one_agent_is_checked_for_it_and_skipped_for_the_others() {
        let world = World::ready()
            .claude_ready()
            .with_agent("codex")
            .with_policy(CLAUDE_POLICY);
        for ask in [None, Some("claude")] {
            let report = world.report(ask);
            let ids: Vec<_> = report.model_routing.iter().map(|c| c.id).collect();
            assert_eq!(
                ids,
                ["policy", "host", "model", "effort", "overrides"],
                "{ask:?}"
            );
            // Every check but the summary of the file says which agent it is about.
            assert!(report.model_routing[1..]
                .iter()
                .all(|c| c.agent == Some("claude")));
        }
        for other in ["codex", "copilot"] {
            let report = world.report(Some(other));
            // The summary of the file, then what the policy does for that agent.
            assert_eq!(report.model_routing.len(), 2, "{other}");
            let checks = &report.model_routing[1..];
            // An agent without an entry is skipped.
            assert_eq!(checks[0].status, Status::Skipped, "{other}");
            assert!(
                checks[0].message.contains(&format!("no entry for {other}")),
                "{other}"
            );
            // Only a missing Copilot command, never the policy, can make that report not ready.
            assert!(checks.iter().all(|c| !c.blocks()), "{other}");
        }
    }

    #[test]
    fn commands_are_read_from_bullets_under_the_commands_heading_only() {
        let text = "## Other\n\n- Skip: `no`\n\n## Commands\n\nRun `not-a-bullet` here.\n\n- Test: `cargo test`\n* Two: `a` and `b`\n\n## Next\n\n- Nope: `c`\n";
        assert_eq!(
            documented_commands(text),
            Some(vec!["cargo test".into(), "a".into(), "b".into()])
        );
        assert_eq!(documented_commands("# No commands\n"), None);
    }

    // --- model routing -----------------------------------------------------------------------------

    const CLAUDE_POLICY: &str = r#"{"agents":{"claude":{"stages":{
        "build":{"model":"sonnet","effort":"medium"},
        "check":{"model":"opus","effort":"high"}}}}}"#;

    impl World {
        /// Writes the user's model-routing policy where doctor looks for it, inside the fake home.
        fn with_policy(mut self, policy: &str) -> Self {
            let config = self.home.path().join("config");
            fs::create_dir_all(config.join("tds-factory")).unwrap();
            fs::write(
                config.join("tds-factory").join("model-routing.json"),
                policy,
            )
            .unwrap();
            for name in ["XDG_CONFIG_HOME", "APPDATA"] {
                self.vars.insert(name.into(), config.clone().into());
            }
            self
        }

        /// Points the configuration folder at a temporary one inside the fake home, with no policy in it.
        fn with_config_folder(mut self) -> Self {
            let config = self.home.path().join("config");
            for name in ["XDG_CONFIG_HOME", "APPDATA"] {
                self.vars.insert(name.into(), config.clone().into());
            }
            self
        }

        fn with_version(mut self, agent: &str, output: &str) -> Self {
            self.versions.insert(agent.into(), output.into());
            self
        }

        fn with_var(mut self, name: &str, value: &str) -> Self {
            self.vars.insert(name.into(), value.into());
            self
        }

        fn claude_ready(self) -> Self {
            self.with_agent("claude")
                .with_version("claude", "2.1.318 (Claude Code)")
        }
    }

    fn routing_check<'a>(report: &'a Report, id: &str) -> &'a Check {
        report
            .model_routing
            .iter()
            .find(|check| check.id == id)
            .unwrap_or_else(|| panic!("no {id} model-routing check in {report:?}"))
    }

    #[test]
    fn without_a_policy_there_is_no_model_routing_section() {
        let report = World::ready().claude_ready().report(None);
        assert!(report.model_routing.is_empty());
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("model_routing").is_none());
        assert!(!render(&report).contains("Model routing"));
    }

    #[test]
    fn a_claude_policy_that_fits_passes_every_check() {
        let report = World::ready()
            .claude_ready()
            .with_policy(CLAUDE_POLICY)
            .report(None);
        assert!(report.ok, "{}", render(&report));
        let ids: Vec<_> = report.model_routing.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["policy", "host", "model", "effort", "overrides"]);
        assert!(
            report.model_routing.iter().all(Check::passed),
            "{}",
            render(&report)
        );
    }

    #[test]
    fn an_old_or_missing_claude_is_a_warning_and_the_factory_stays_ready() {
        let old = World::ready()
            .with_agent("claude")
            .with_version("claude", "2.1.291 (Claude Code)")
            .with_policy(CLAUDE_POLICY)
            .report(None);
        let host = routing_check(&old, "host");
        assert_eq!(host.status, Status::Warning);
        assert!(host.message.contains("2.1.291") && host.message.contains("2.1.292"));
        assert!(host.fix.as_deref().unwrap().contains("2.1.292"));
        assert!(old.ok, "{}", render(&old));

        // The command is on PATH for one agent only; for claude it is missing.
        let missing = World::ready()
            .with_agent("codex")
            .with_policy(CLAUDE_POLICY)
            .report(None);
        assert_eq!(routing_check(&missing, "host").status, Status::Warning);
        assert!(routing_check(&missing, "host")
            .message
            .contains("not on PATH"));

        // A version that cannot be read is not guessed at.
        let unreadable = World::ready()
            .with_agent("claude")
            .with_version("claude", "something unexpected")
            .with_policy(CLAUDE_POLICY)
            .report(None);
        assert_eq!(routing_check(&unreadable, "host").status, Status::Skipped);
    }

    #[test]
    fn models_the_agent_tool_does_not_accept_are_named_per_stage() {
        let policy = r#"{"agents":{"claude":{"stages":{
            "check":{"model":"claude-opus-5-5","effort":"high"},"build":{"model":"sonnet","effort":"low"}}}}}"#;
        let report = World::ready()
            .claude_ready()
            .with_policy(policy)
            .report(None);
        let model = routing_check(&report, "model");
        assert_eq!(model.status, Status::Warning);
        assert!(
            model.message.contains("check: claude-opus-5-5"),
            "{}",
            model.message
        );
        assert!(!model.message.contains("build"), "{}", model.message);
        // The effort is fine, so it is reported separately.
        assert_eq!(routing_check(&report, "effort").status, Status::Pass);
        assert!(report.ok);
    }

    #[test]
    fn unsupported_and_missing_efforts_are_reported_apart_from_models() {
        let policy = r#"{"agents":{"claude":{"stages":{
            "check":{"model":"opus","effort":"extreme"},"build":{"model":"sonnet"}}}}}"#;
        let report = World::ready()
            .claude_ready()
            .with_policy(policy)
            .report(None);
        assert_eq!(routing_check(&report, "model").status, Status::Pass);
        let effort = routing_check(&report, "effort");
        assert_eq!(effort.status, Status::Warning);
        assert!(
            effort.message.contains("check: extreme"),
            "{}",
            effort.message
        );
        assert!(effort.message.contains("no effort set") && effort.message.contains("build"));
        // A stage left out of the policy is not judged.
        assert!(!effort.message.contains("define"), "{}", effort.message);
    }

    #[test]
    fn variables_that_override_the_policy_are_found_wherever_they_are_set() {
        let ready = || World::ready().claude_ready().with_policy(CLAUDE_POLICY);
        let overrides = |world: &World| routing_check(&world.report(None), "overrides").status;

        // In the environment.
        let forced = ready()
            .with_var("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "1")
            .with_var("CLAUDE_CODE_SUBAGENT_MODEL", "haiku");
        let report = forced.report(None);
        let check = routing_check(&report, "overrides");
        assert_eq!(check.status, Status::Warning);
        assert!(
            check.message.contains("every subagent on haiku"),
            "{}",
            check.message
        );
        assert!(report.ok);

        let effort = ready().with_var("CLAUDE_CODE_EFFORT_LEVEL", "low");
        let report = effort.report(None);
        assert!(routing_check(&report, "overrides")
            .message
            .contains("CLAUDE_CODE_EFFORT_LEVEL=low (the environment)"));

        // Neither variable alone on the model side overrides the Agent tool's own choice.
        assert_eq!(
            overrides(&ready().with_var("CLAUDE_CODE_SUBAGENT_MODEL", "haiku")),
            Status::Pass
        );
        assert_eq!(
            overrides(&ready().with_var("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "1")),
            Status::Pass
        );
        assert_eq!(
            overrides(
                &ready()
                    .with_var("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "0")
                    .with_var("CLAUDE_CODE_SUBAGENT_MODEL", "haiku")
            ),
            Status::Pass
        );

        // In Claude Code's user settings and the project's settings files.
        let world = ready();
        let claude_dir = world.home.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.json"),
            r#"{"env":{"CLAUDE_CODE_EFFORT_LEVEL":"max"}}"#,
        )
        .unwrap();
        let report = world.report(None);
        assert!(routing_check(&report, "overrides")
            .message
            .contains("(user settings)"));

        let world = ready();
        let project_dir = world.project.path().join(".claude");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(
            project_dir.join("settings.local.json"),
            r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL_FORCE":"1","CLAUDE_CODE_SUBAGENT_MODEL":"sonnet"}}"#,
        )
        .unwrap();
        let report = world.report(None);
        assert!(routing_check(&report, "overrides")
            .message
            .contains("(local project settings)"));
    }

    #[test]
    fn settings_files_that_cannot_be_read_are_skipped_not_passed() {
        let world = World::ready().claude_ready().with_policy(CLAUDE_POLICY);
        let project_dir = world.project.path().join(".claude");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("settings.json"), "{ not json").unwrap();
        let report = world.report(None);
        let check = routing_check(&report, "overrides");
        assert_eq!(check.status, Status::Skipped);
        assert!(check.message.contains("settings.json"), "{}", check.message);
    }

    #[test]
    fn a_codex_policy_reports_what_it_can_and_skips_the_rest() {
        let policy = r#"{"agents":{"codex":{"stages":{"build":{"model":"gpt-6.1-sol","effort":"medium"}}}}}"#;
        let report = World::ready()
            .with_agent("codex")
            .with_version("codex", "codex-cli 0.99.1")
            .with_policy(policy)
            .report(None);
        assert!(report.ok, "{}", render(&report));
        let host = routing_check(&report, "host");
        assert_eq!(host.status, Status::Skipped);
        assert!(host.message.contains("0.99.1"), "{}", host.message);
        assert_eq!(routing_check(&report, "model").status, Status::Skipped);
        assert_eq!(routing_check(&report, "overrides").status, Status::Skipped);
        assert!(report
            .model_routing
            .iter()
            .all(|c| c.status != Status::Fail));

        let missing = World::ready().with_policy(policy).report(None);
        assert_eq!(routing_check(&missing, "host").status, Status::Warning);
    }

    #[test]
    fn a_copilot_entry_checks_the_command_models_and_effort_and_warns_about_problems() {
        let policy = r#"{"agents":{"copilot":{"stages":{
            "check":{"model":"claude-opus-4.8","effort":"high"}}}}}"#;
        let report = World::ready()
            .claude_ready()
            .with_agent("copilot")
            .with_policy(policy)
            .report(Some("copilot"));
        let copilot = tagged(&report, "copilot");
        let ids: Vec<_> = copilot.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["host", "model", "effort", "overrides"]);
        assert!(copilot
            .iter()
            .any(|c| c.id == "model" && c.status == Status::Pass));
        assert!(copilot.iter().all(|c| !c.blocks()));
        // A missing effort is a warning, never a failure.
        let weak = r#"{"agents":{"copilot":{"stages":{
            "check":{"model":"claude-opus-4.8"},"build":{"model":"claude-sonnet-5.5","effort":"low"}}}}}"#;
        let report = World::ready()
            .claude_ready()
            .with_agent("copilot")
            .with_policy(weak)
            .report(Some("copilot"));
        assert!(report.ok, "{}", render(&report));
        let warned: Vec<_> = tagged(&report, "copilot")
            .iter()
            .filter(|c| c.status == Status::Warning)
            .map(|c| c.id)
            .collect();
        assert_eq!(warned, ["effort"]);
        // A policy that is switched off says nothing.
        let off = World::ready()
            .claude_ready()
            .with_policy(r#"{"enabled":false,"agent":"claude"}"#)
            .report(None);
        assert!(off.model_routing.is_empty());
    }

    #[test]
    fn a_policy_file_that_stops_run_start_makes_the_report_not_ready() {
        let policy = r#"{"agents":{"claude":{"stages":{"check":{"model":"a b"}}}}}"#;
        let report = World::ready()
            .claude_ready()
            .with_policy(policy)
            .report(None);
        assert!(!report.ok);
        assert_eq!(report.model_routing.len(), 1);
        let check = routing_check(&report, "policy");
        assert_eq!(check.status, Status::Fail);
        assert!(check.message.contains("model-routing.json") && check.message.contains("Model"));
        assert!(check.fix.as_deref().unwrap().contains("enabled"));
    }

    #[test]
    fn an_old_format_policy_fails_the_report_and_says_how_to_replace_it() {
        let old = r#"{"agents":{"claude":{"roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}}}}"#;
        let report = World::ready().claude_ready().with_policy(old).report(None);
        assert!(!report.ok);
        let check = routing_check(&report, "policy");
        assert_eq!(check.status, Status::Fail);
        assert!(
            check.message.contains("old format") && check.message.contains("factory install"),
            "{}",
            check.message
        );
    }

    #[test]
    fn the_report_shows_model_routing_in_its_own_section_with_the_new_statuses() {
        let report = World::ready()
            .with_agent("claude")
            .with_version("claude", "2.1.291")
            .with_policy(CLAUDE_POLICY)
            .report(None);
        let text = render(&report);
        let section = text.split("Model routing\n").nth(1).expect(&text);
        assert!(section.contains("  pass     policy:"), "{text}");
        assert!(section.contains("  warning  host [claude]:"), "{text}");
        assert!(
            section.contains("Fix: Update claude to 2.1.292 or later."),
            "{text}"
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["model_routing"][1]["status"], "warning");
        assert!(text.contains("Ready"), "{text}");
    }

    /// A repository can name anything as one of the files doctor reads, such as a device that never ends.
    #[cfg(unix)]
    #[test]
    fn only_regular_files_are_read_from_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let device = dir.path().join("settings.json");
        std::os::unix::fs::symlink("/dev/null", &device).unwrap();
        assert!(read_doc(&device).is_err());
        assert!(read_doc(dir.path()).is_err());
        let plain = dir.path().join("plain.json");
        fs::write(&plain, "{}").unwrap();
        assert_eq!(read_doc(&plain).unwrap(), "{}");
        fs::write(&plain, vec![b' '; MAX_DOC_BYTES as usize + 1]).unwrap();
        assert!(read_doc(&plain).is_err());
    }

    /// Opening a pipe for reading waits for a writer, so it must be refused before it is opened.
    #[cfg(unix)]
    #[test]
    fn a_named_pipe_in_the_repository_is_refused_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let pipe = dir.path().join("settings.local.json");
        assert!(Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap()
            .success());
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&pipe, &link).unwrap();
        for path in [pipe, link] {
            let (sender, receiver) = std::sync::mpsc::channel();
            // A thread that is still waiting when the test ends is cleaned up with the process.
            std::thread::spawn(move || sender.send(read_doc(&path).is_err()));
            let refused = receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("read_doc waited for a writer on a named pipe");
            assert!(refused);
        }
    }

    #[test]
    fn a_relative_program_is_never_run_to_ask_its_version() {
        assert_eq!(agent_version(Path::new("claude")), None);
        assert_eq!(agent_version(Path::new("./claude")), None);
    }

    #[test]
    fn a_stage_without_a_model_is_a_policy_failure() {
        let policy = r#"{"agents":{"claude":{"stages":{"build":{"effort":"low"}}}}}"#;
        let report = World::ready()
            .claude_ready()
            .with_policy(policy)
            .report(None);
        assert!(!report.ok);
        let check = routing_check(&report, "policy");
        assert_eq!(check.status, Status::Fail);
        assert!(check.message.contains("model"), "{}", check.message);
    }

    #[test]
    fn versions_are_read_from_the_first_dotted_number() {
        assert_eq!(
            routing::parse_version("2.1.318 (Claude Code)"),
            Some((2, 1, 318))
        );
        assert_eq!(
            routing::parse_version("codex-cli v0.99.1\n"),
            Some((0, 99, 1))
        );
        assert_eq!(routing::parse_version("version 2.1"), None);
        assert_eq!(routing::parse_version(""), None);
    }

    fn provider_check<'a>(report: &'a Report, id: &str) -> Option<&'a Check> {
        report.provider.iter().find(|check| check.id == id)
    }

    #[test]
    fn windows_skips_the_extensionless_launcher_next_to_the_cmd_one() {
        let bin = tempfile::tempdir().unwrap();
        for name in ["az", "az.cmd"] {
            let file = bin.path().join(name);
            fs::write(&file, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let extensions = windows_extensions(".COM;.EXE;;.cmd");
        assert_eq!(extensions, [".COM", ".EXE", ".cmd"]);
        let found = find_with_extensions("az", &bin.path().into(), &extensions).unwrap();
        assert_eq!(found, bin.path().join("az.cmd"));
    }

    #[test]
    fn a_name_with_a_pathext_extension_is_also_tried_as_given() {
        let extensions = windows_extensions(".COM;.EXE;.CMD");
        assert_eq!(
            candidate_suffixes("npm.cmd", &extensions),
            ["", ".COM", ".EXE", ".CMD"]
        );
        assert_eq!(
            candidate_suffixes("tool.EXE", &extensions),
            ["", ".COM", ".EXE", ".CMD"]
        );
        assert_eq!(
            candidate_suffixes("az", &extensions),
            [".COM", ".EXE", ".CMD"]
        );
        assert_eq!(candidate_suffixes("git", &[String::new()]), [""]);
    }

    /// Runs on Windows in CI: names that carry their extension resolve, and bare `az` still skips the launcher.
    #[cfg(windows)]
    #[test]
    fn windows_finds_names_given_with_their_extension() {
        let bin = tempfile::tempdir().unwrap();
        for name in ["tool.exe", "npm.cmd", "az", "az.cmd"] {
            fs::write(bin.path().join(name), "").unwrap();
        }
        let path: OsString = bin.path().into();
        let lower = |p: PathBuf| p.to_string_lossy().to_lowercase();
        for name in ["tool.exe", "npm.cmd", "npm.CMD"] {
            let found = find_program(name, &path).unwrap_or_else(|| panic!("{name} not found"));
            assert_eq!(lower(found), lower(bin.path().join(name)));
        }
        let az = find_program("az", &path).unwrap();
        assert_eq!(lower(az), lower(bin.path().join("az.cmd")));
    }

    /// Runs on Windows in CI: the Bash launcher must not shadow `az.cmd`, and the `.cmd` must start.
    #[cfg(windows)]
    #[test]
    fn az_resolves_to_the_cmd_launcher_and_the_cmd_runs() {
        let bin = tempfile::tempdir().unwrap();
        fs::write(bin.path().join("az"), "#!/usr/bin/env bash\nexit 1\n").unwrap();
        fs::write(
            bin.path().join("az.cmd"),
            "@echo off\r\necho launched %1\r\n",
        )
        .unwrap();
        let program = az_program_in(&bin.path().into());
        // PATHEXT spells the extension in capitals, which the file system treats as the same file.
        assert_eq!(
            program.to_string_lossy().to_lowercase(),
            bin.path().join("az.cmd").to_string_lossy().to_lowercase()
        );
        let output = Command::new(&program).arg("version").output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "launched version"
        );
    }

    /// A bare `az` is looked up by Windows as `az.exe` only, so it never finds the Azure CLI there.
    #[test]
    fn az_is_never_started_by_its_bare_name() {
        let bare = ["Command::new(", "\"az\")"].concat();
        let mut found = Vec::new();
        let mut folders = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(folder) = folders.pop() {
            for entry in fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && fs::read_to_string(&path).unwrap().contains(&bare)
                {
                    found.push(path);
                }
            }
        }
        assert!(
            found.is_empty(),
            "start the Azure CLI with `az_program()`, not by name: {found:?}"
        );
    }

    /// A fake host tool on PATH: a shell script that exits 0 only for the listed argument lines. Tests that
    /// use it run on Unix only, because the fake is a `#!/bin/sh` script.
    #[cfg(unix)]
    fn fake_tool(bin: &Path, name: &str, succeeds_for: &[&str]) {
        use std::os::unix::fs::PermissionsExt;
        let cases: String = succeeds_for
            .iter()
            .map(|args| format!("  \"{args}\") exit 0 ;;\n"))
            .collect();
        let file = bin.join(name);
        fs::write(
            &file,
            format!("#!/bin/sh\ncase \"$*\" in\n{cases}esac\necho secret-output\nexit 1\n"),
        )
        .unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn with_origin(world: World, url: &str) -> World {
        git(world.project.path(), &["remote", "set-url", "origin", url]);
        world
    }

    #[test]
    fn a_github_origin_without_gh_is_a_warning_that_keeps_the_project_ready() {
        let world = with_origin(
            World::ready().claude_ready(),
            "https://github.com/me/repo.git",
        );
        let report = world.report(None);
        assert!(report.ok, "{}", render(&report));
        assert_eq!(
            provider_check(&report, "host").unwrap().status,
            Status::Pass
        );
        assert!(provider_check(&report, "host")
            .unwrap()
            .message
            .contains("GitHub"));
        assert_eq!(
            provider_check(&report, "gh").unwrap().status,
            Status::Warning
        );
        assert!(provider_check(&report, "sign-in").is_none());
        assert!(render(&report).contains("\nProvider\n"));
    }

    #[cfg(unix)]
    #[test]
    fn a_github_origin_reports_gh_and_its_sign_in() {
        let world = with_origin(World::ready(), "git@github.com:me/repo.git");
        fake_tool(world.bin.path(), "gh", &["auth status"]);
        let report = world.report(None);
        assert_eq!(provider_check(&report, "gh").unwrap().status, Status::Pass);
        assert_eq!(
            provider_check(&report, "sign-in").unwrap().status,
            Status::Pass
        );
        let not_verified = provider_check(&report, "not-verified").unwrap();
        assert_eq!(not_verified.status, Status::Skipped);
        assert!(not_verified.message.contains("token is valid"));
        assert!(not_verified.message.contains("branch policies"));

        let signed_out = with_origin(World::ready(), "git@github.com:me/repo.git");
        fake_tool(signed_out.bin.path(), "gh", &[]);
        let report = signed_out.report(None);
        let sign_in = provider_check(&report, "sign-in").unwrap();
        assert_eq!(sign_in.status, Status::Warning);
        assert!(sign_in.fix.as_deref().unwrap().contains("gh auth login"));
    }

    const AZURE_ORIGIN: &str = "https://me@dev.azure.com/org/My%20Project/_git/repo";

    #[test]
    fn an_azure_origin_without_az_is_a_warning() {
        let world = with_origin(World::ready().claude_ready(), AZURE_ORIGIN);
        let report = world.report(None);
        assert!(report.ok, "{}", render(&report));
        let host = provider_check(&report, "host").unwrap();
        assert_eq!(host.status, Status::Pass);
        assert!(host.message.contains("Azure Repos (org/My Project/repo)"));
        assert_eq!(
            provider_check(&report, "az").unwrap().status,
            Status::Warning
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_azure_origin_reports_the_extension_and_the_sign_in() {
        let no_extension = with_origin(World::ready(), AZURE_ORIGIN);
        fake_tool(no_extension.bin.path(), "az", &["account show"]);
        let report = no_extension.report(None);
        assert_eq!(provider_check(&report, "az").unwrap().status, Status::Pass);
        let extension = provider_check(&report, "azure-devops-extension").unwrap();
        assert_eq!(extension.status, Status::Warning);
        assert!(extension
            .fix
            .as_deref()
            .unwrap()
            .contains("az extension add --name azure-devops"));
        // The sign-in needs no extension, so it is still reported.
        assert_eq!(
            provider_check(&report, "sign-in").unwrap().status,
            Status::Pass
        );

        let signed_out = with_origin(World::ready(), AZURE_ORIGIN);
        fake_tool(
            signed_out.bin.path(),
            "az",
            &["extension show --name azure-devops"],
        );
        let report = signed_out.report(None);
        assert_eq!(
            provider_check(&report, "azure-devops-extension")
                .unwrap()
                .status,
            Status::Pass
        );
        let sign_in = provider_check(&report, "sign-in").unwrap();
        assert_eq!(sign_in.status, Status::Warning);
        assert!(sign_in.message.contains("AZURE_DEVOPS_EXT_PAT is not set"));

        let signed_in = with_origin(World::ready(), AZURE_ORIGIN);
        fake_tool(
            signed_in.bin.path(),
            "az",
            &["extension show --name azure-devops", "account show"],
        );
        let report = signed_in.report(None);
        assert_eq!(
            provider_check(&report, "sign-in").unwrap().status,
            Status::Pass
        );
        assert!(!render(&report).contains("secret-output"));
    }

    #[cfg(unix)]
    #[test]
    fn a_personal_access_token_counts_as_signed_in_and_is_never_shown() {
        let mut world = with_origin(World::ready(), AZURE_ORIGIN);
        world
            .vars
            .insert("AZURE_DEVOPS_EXT_PAT".into(), "tok3n-value".into());
        fake_tool(
            world.bin.path(),
            "az",
            &["extension show --name azure-devops"],
        );
        let report = world.report(None);
        let sign_in = provider_check(&report, "sign-in").unwrap();
        assert_eq!(sign_in.status, Status::Pass);
        assert_eq!(sign_in.message, "AZURE_DEVOPS_EXT_PAT is set");
        assert!(!render(&report).contains("tok3n-value"));
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("tok3n-value"));
    }

    #[test]
    fn an_unknown_host_is_a_warning_with_what_to_do_instead() {
        let report = World::ready().claude_ready().report(None);
        assert!(report.ok, "{}", render(&report));
        assert_eq!(report.provider.len(), 1);
        let host = provider_check(&report, "host").unwrap();
        assert_eq!(host.status, Status::Warning);
        assert!(host.message.contains("GitHub or Azure Repos"));
        assert!(host.fix.as_deref().unwrap().contains("--pr <url>"));
    }

    #[test]
    fn an_unknown_provider_override_is_reported_as_unsupported() {
        let world = World::ready().claude_ready();
        git(
            world.project.path(),
            &["config", "factory.prProvider", "example"],
        );
        let report = world.report(None);
        assert!(report.ok, "{}", render(&report));
        let host = provider_check(&report, "host").unwrap();
        assert_eq!(host.status, Status::Warning);
        assert!(host.message.contains("not supported"), "{}", host.message);
        assert!(host.message.contains("choose github or azure"));
    }

    #[test]
    fn an_azure_override_that_does_not_match_origin_is_reported_as_unsupported() {
        let world = with_origin(World::ready().claude_ready(), AZURE_ORIGIN);
        for (key, value) in [
            ("factory.prProvider", "azure"),
            ("factory.azureOrganization", "other-org"),
            ("factory.azureProject", "My Project"),
            ("factory.azureRepository", "repo"),
        ] {
            git(world.project.path(), &["config", key, value]);
        }
        let report = world.report(None);
        assert!(report.ok, "{}", render(&report));
        let host = provider_check(&report, "host").unwrap();
        assert_eq!(host.status, Status::Warning);
        assert!(
            host.message.contains("does not match origin"),
            "{}",
            host.message
        );
    }

    #[test]
    fn without_origin_the_provider_is_not_checked() {
        let world = World::ready();
        git(world.project.path(), &["remote", "remove", "origin"]);
        let report = world.report(None);
        assert_eq!(report.provider.len(), 1);
        assert_eq!(
            provider_check(&report, "host").unwrap().status,
            Status::Skipped
        );
    }
}
