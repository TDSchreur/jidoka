//! `factory doctor`: a read-only report on whether this project and this machine are ready for the factory.
//! It only reads files, looks on `PATH` and asks `git` read-only questions. It never runs the project's own
//! commands, installs anything or needs the dashboard.

use crate::install::{bundled_skill_names, skills_root};
use serde::Serialize;
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

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
}

#[derive(Serialize, Debug)]
pub(crate) struct Check {
    id: &'static str,
    status: Status,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<String>,
}

impl Check {
    fn pass(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: Status::Pass,
            message: message.into(),
            fix: None,
        }
    }
    fn missing(id: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            id,
            status: Status::Missing,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }
    fn fail(id: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            id,
            status: Status::Fail,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }
    fn passed(&self) -> bool {
        self.status == Status::Pass
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
}

/// Everything the checks look at besides the project folder, so tests can supply their own.
pub(crate) struct Machine<'a> {
    pub(crate) home: PathBuf,
    pub(crate) path: OsString,
    pub(crate) var: &'a dyn Fn(&str) -> Option<OsString>,
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
    let git = git_checks(project);
    // Later checks read from the work tree root, so they work from a subfolder or a linked worktree.
    let root = git_output(project, &["rev-parse", "--show-toplevel"])
        .map_or_else(|| project.to_path_buf(), PathBuf::from);
    let mut project_checks = git;
    project_checks.push(setup_docs_check(&root));
    project_checks.push(verification_check(&root, &machine.path));
    let agents: Vec<AgentReport> = agent_ids
        .into_iter()
        .map(|id| {
            let checks = vec![
                executable_check(id, &machine.path),
                skills_check(id, machine),
            ];
            AgentReport {
                id,
                usable: checks.iter().all(Check::passed),
                checks,
            }
        })
        .collect();
    let shared_ok = project_checks.iter().all(Check::passed);
    let agents_ok = match agent {
        Some(_) => agents.iter().all(|a| a.usable),
        None => agents.iter().any(|a| a.usable),
    };
    Ok(Report {
        ok: shared_ok && agents_ok,
        project: project_checks,
        agents,
    })
}

fn git_output(project: &Path, args: &[&str]) -> Option<String> {
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
        Some(url) => checks.push(Check::pass("origin", format!("origin is {url}"))),
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

fn read_doc(path: &Path) -> io::Result<String> {
    if fs::metadata(path)?.len() > MAX_DOC_BYTES {
        return Err(io::Error::other("larger than 1 MiB"));
    }
    fs::read_to_string(path)
}

fn setup_docs_check(root: &Path) -> Check {
    const FIX: &str = "Run `/setup-matt-pocock-skills` once in the project and commit its output.";
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
            format!("the factory setup is incomplete: {}", missing.join(", ")),
            FIX,
        )
    }
}

/// The commands documented as bullets under `## Commands`, e.g. ``- Test: `cargo test` ``.
fn documented_commands(text: &str) -> Option<Vec<String>> {
    let mut lines = text
        .lines()
        .skip_while(|line| !line.trim_end().starts_with("## Commands"));
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

/// Finds `name` in the `PATH` folders; on Windows it also tries the `PATHEXT` extensions.
fn find_program(name: &str, path: &OsString) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        std::iter::once(String::new())
            .chain(
                std::env::var("PATHEXT")
                    .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
                    .split(';')
                    .filter(|ext| ext.starts_with('.'))
                    .map(str::to_string),
            )
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(path).find_map(|dir| {
        extensions.iter().find_map(|ext| {
            let candidate = dir.join(format!("{name}{ext}"));
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

fn skills_check(agent: &'static str, machine: &Machine) -> Check {
    let root = match skills_root(agent, &machine.home, machine.var) {
        Ok(root) => root,
        Err(error) => {
            return Check::fail(
                "skills",
                error.to_string(),
                "Use codex, claude, or copilot.",
            )
        }
    };
    let absent: Vec<&str> = bundled_skill_names()
        .into_iter()
        .filter(|name| !root.join(name).join("SKILL.md").is_file())
        .collect();
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
            format!("Run `factory install --agent {agent}`."),
        )
    }
}

/// The human-readable report.
pub(crate) fn render(report: &Report) -> String {
    fn line(check: &Check, indent: &str) -> String {
        let mut text = format!(
            "{indent}{:<8} {}: {}\n",
            format!("{:?}", check.status).to_lowercase(),
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
    out.push_str(if report.ok {
        "\nReady: the factory can run here.\n"
    } else {
        "\nNot ready: fix the items above and run `factory doctor` again.\n"
    });
    out
}

/// Runs the checks for the current folder and prints the report. Returns whether the factory is ready.
pub(crate) fn run(agent: Option<&str>, json: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let machine = Machine {
        home: crate::install::home_dir()?,
        path: std::env::var_os("PATH").unwrap_or_default(),
        var: &|name| std::env::var_os(name),
    };
    let report = diagnose(&std::env::current_dir()?, agent, &machine)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render(&report));
    }
    Ok(report.ok)
}

/// Reads `[--agent <name>] [--json]` in any order.
pub(crate) fn parse_args<'a>(args: &[&'a str]) -> Result<(Option<&'a str>, bool), String> {
    let (mut agent, mut json) = (None, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--json" => json = true,
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
    Ok((agent, json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    /// A project, a home folder and a `PATH` folder, all temporary.
    struct World {
        project: TempDir,
        home: TempDir,
        bin: TempDir,
        vars: HashMap<String, OsString>,
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
        for name in bundled_skill_names() {
            fs::create_dir_all(root.join(name)).unwrap();
            fs::write(root.join(name).join("SKILL.md"), "skill").unwrap();
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
            let machine = Machine {
                home: self.home.path().to_path_buf(),
                path: self.bin.path().into(),
                var: &|name| self.vars.get(name).cloned(),
            };
            diagnose(self.project.path(), agent, &machine).unwrap()
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
        let world = world.with_agent("claude").with_agent("codex");
        assert!(custom
            .path()
            .join("skills")
            .join(bundled_skill_names()[0])
            .is_dir());
        assert!(custom.path().join("codex/skills").is_dir());
        assert!(!world.home.path().join(".claude").exists());
        let report = world.report(None);
        assert!(agent(&report, "claude").usable);
        assert!(agent(&report, "codex").usable);
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
        assert!(docs
            .fix
            .as_ref()
            .unwrap()
            .contains("/setup-matt-pocock-skills"));

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
        assert_eq!(parse_args(&[]), Ok((None, false)));
        assert_eq!(
            parse_args(&["--json", "--agent", "copilot"]),
            Ok((Some("copilot"), true))
        );
        assert_eq!(
            parse_args(&["--agent", "codex"]),
            Ok((Some("codex"), false))
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
    fn commands_are_read_from_bullets_under_the_commands_heading_only() {
        let text = "## Other\n\n- Skip: `no`\n\n## Commands\n\nRun `not-a-bullet` here.\n\n- Test: `cargo test`\n* Two: `a` and `b`\n\n## Next\n\n- Nope: `c`\n";
        assert_eq!(
            documented_commands(text),
            Some(vec!["cargo test".into(), "a".into(), "b".into()])
        );
        assert_eq!(documented_commands("# No commands\n"), None);
    }
}
