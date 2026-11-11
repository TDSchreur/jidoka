//! `jidoka doctor`: a report on whether this project and this machine are ready for Jidoka.
//! It only reads files, looks on `PATH` and asks `git` read-only questions, asks the pull request host's tool
//! whether it is signed in (`gh auth status`, `az extension show`, `az account show`), asks `rustup target list --installed`
//! in a Rust project on a non-Windows host (read-only), and, when a model-routing policy exists, asks an agent for its `--version`. It never runs the project's own commands, calls a model,
//! installs anything or needs the dashboard. The one thing it writes is a starter model-routing policy, and
//! only when `--create-model-routing` asks for it.

use crate::install::{bundled_skill_files, claude_config_dirs, skills_root};
use crate::routing::{RoutingAgent, create_starter, created_message, policy_path_in};
use serde::Serialize;
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

mod provider;
mod routing;

use crate::install::{AGENTS, agent_list};

/// Project files are not trusted: anything larger is not read.
const MAX_DOC_BYTES: u64 = 1 << 20;
/// How much of an agent's `--version` output doctor shows: a version line is short, so this only guards against
/// a program that prints something huge.
const VERSION_OUTPUT_CHARS: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Pass,
    /// Something the user can supply: a file, a tool, an installed skill.
    Missing,
    /// Something is broken or cannot be read.
    Fail,
    /// A risk to read about that does not stop Jidoka, such as model routing that will not apply.
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
    /// Whether this check makes Jidoka not ready. Warnings and skipped checks never do.
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
                format!("unknown agent {name:?}; use {}", agent_list()),
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
    let skills_version = skills_version_check(machine);
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
    project_checks.extend(skills_version);
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
            )];
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
    const FIX: &str = "Run `/jidoka-setup` once in the project and commit its output.";
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
                );
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
                "Jidoka setup is incomplete: {} (needed by /jidoka-full, and by the lighter workflows when a run reads or publishes a spec or tickets)",
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
            );
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
            format!("Use {}.", agent_list()),
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
            format!("all jidoka skills are installed in {}", root.display()),
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
                    "Run `jidoka install --agent claude --config-dir {}`.",
                    dir.display()
                )
            } else {
                format!("Run `jidoka install --agent {agent}`.")
            },
        )
    }
}

/// Compares the install record's version with this binary. No (or an unreadable) record adds no check.
/// A warning never blocks: the next `jidoka` command refreshes the skills by itself.
fn skills_version_check(machine: &Machine) -> Option<Check> {
    let path = crate::install_record::path_in(machine.var)?;
    let record = crate::install_record::read(&path).ok()??;
    let binary = env!("CARGO_PKG_VERSION");
    Some(if record.version == binary {
        Check::pass(
            "skills-version",
            format!("skills match this jidoka ({binary})"),
        )
    } else {
        let installed = if record.version.is_empty() {
            "an unknown version"
        } else {
            &record.version
        };
        Check::warning(
            "skills-version",
            format!("skills are outdated: installed from {installed}, this jidoka is {binary}"),
            "Run any `jidoka` command or `jidoka install`.",
        )
    })
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
        "\nReady: Jidoka can run here.\n"
    } else {
        "\nNot ready: fix the items above and run `jidoka doctor` again.\n"
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
            .take(VERSION_OUTPUT_CHARS)
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

/// Runs the checks for the current folder and prints the report. Returns whether Jidoka is ready.
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

/// What `jidoka doctor` was asked to do.
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
    match create_starter(&path, agents) {
        Ok(true) => {
            let names: Vec<_> = agents.iter().map(|agent| agent.name()).collect();
            let named = match names.split_last() {
                Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
                _ => names.join(""),
            };
            let about = format!(
                "It is a starter policy for {named} that routes every stage (define, build, check, finish) of each."
            );
            Ok(created_message(&shown, Some(&about)))
        }
        // A link that points nowhere is "there" for creating but not for reading, so say which it is.
        Ok(false) if fs::metadata(&path).is_err() => Ok(format!(
            "{shown} is a link that points nowhere, so no policy is read from it. It was left unchanged."
        )),
        Ok(false) => Ok(format!("{shown} already exists and was left unchanged.")),
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
                    return Err("--agent can be given only once.".into());
                }
                Some(name) => {
                    return Err(format!("Unknown agent {name:?}; use {}.", agent_list()));
                }
                None => return Err(format!("--agent needs {}.", agent_list())),
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
mod tests;
