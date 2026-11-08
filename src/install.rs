use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));

/// Skill directories from earlier releases that are no longer bundled: the pre-rename
/// `factory2` skills and development-only skills that were once installed.
const LEGACY_SKILLS: [&str; 4] = [
    "factory2",
    "factory2-fast",
    "github-actions-hardening",
    "rust-best-practices",
];

/// Skills maintained in this project, installed without a third-party license.
const LOCAL_SKILLS: [&str; 7] = [
    "factory",
    "factory-full",
    "factory-procedures",
    "factory-fast",
    "factory-balanced",
    "implement-spec-fast",
    "implement-spec-balanced",
];

/// Reads `[--agent <list>] [--config-dir <path>]...` in any order. `--agent` can be given once.
pub(crate) fn parse_options<'a>(
    options: &[&'a str],
) -> Result<(Option<&'a str>, Vec<String>), String> {
    let (mut agent, mut dirs) = (None, Vec::new());
    let mut options = options.iter();
    while let Some(option) = options.next() {
        match (*option, options.next()) {
            ("--agent", Some(value)) if agent.is_none() => agent = Some(*value),
            ("--config-dir", Some(value)) => dirs.push(value.to_string()),
            (other, _) => return Err(format!("Invalid option {other:?}.")),
        }
    }
    Ok((agent, dirs))
}

/// Installs the bundled skills for each agent in a comma-separated list such as `codex,claude`.
/// `config_dirs` are the `--config-dir` values: they replace the Claude config directory, so the
/// skills go into each of them, and they need `claude` in the list.
pub fn install(agents: &str, config_dirs: &[String]) -> io::Result<()> {
    let roots = skills_roots(agents, config_dirs, &home_dir()?, |name| env::var_os(name))?;
    for root in roots {
        install_to(&root)?;
    }
    ensure_default_policy(|name| env::var_os(name))?;
    Ok(())
}

/// Create the initial routing policy without replacing any existing user configuration.
pub(crate) fn ensure_default_policy(var: impl Fn(&str) -> Option<OsString>) -> io::Result<()> {
    let path = crate::routing::policy_path_in(var).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot find the factory config directory for model routing",
        )
    })?;
    let agents = [
        crate::routing::RoutingAgent::Claude,
        crate::routing::RoutingAgent::Codex,
    ];
    let created = crate::routing::create_starter(&path, &agents).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot create model routing policy {}: {error}",
                path.display()
            ),
        )
    })?;
    if created {
        println!("Created default model routing policy: {}", path.display());
    }
    Ok(())
}

/// Resolves the skills folder of every listed agent before anything is installed.
fn skills_roots(
    agents: &str,
    config_dirs: &[String],
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<Vec<PathBuf>> {
    let mut names: Vec<&str> = Vec::new();
    for name in agents.split(',').map(str::trim) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if !config_dirs.is_empty() && !names.contains(&"claude") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--config-dir applies to the claude agent; add claude to --agent",
        ));
    }
    let mut roots = Vec::new();
    for name in names {
        if name == "claude" {
            roots.extend(
                claude_config_dirs(config_dirs, home, &var)?
                    .into_iter()
                    .map(|dir| dir.join("skills")),
            );
        } else {
            roots.push(skills_root(name, home, &var)?);
        }
    }
    Ok(roots)
}

/// The Claude config directories to use. The source of truth for the precedence: the `--config-dir`
/// values, else `CLAUDE_CONFIG_DIR` (empty counts as unset), else `~/.claude`. A leading `~` expands to
/// `home`, a relative path is an error, and a directory named twice appears once.
pub(crate) fn claude_config_dirs(
    flags: &[String],
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<Vec<PathBuf>> {
    if flags.is_empty() {
        return Ok(vec![agent_home("claude", home, var)?]);
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for flag in flags {
        let dir = expand_config_dir(flag, home)?;
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    Ok(dirs)
}

/// What can follow a leading `~`: `/`, and also `\` on Windows.
#[cfg(windows)]
const SEPARATORS: [char; 2] = ['/', '\\'];
#[cfg(not(windows))]
const SEPARATORS: [char; 1] = ['/'];

/// An absolute path for tests on any system: `/name` on Unix, `C:\name` on Windows.
#[cfg(test)]
pub(crate) fn test_abs(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:\\{}", name.replace('/', "\\")))
    } else {
        PathBuf::from(format!("/{name}"))
    }
}

/// An absolute config directory from a user-given path, expanding a leading `~`.
pub(crate) fn expand_config_dir(value: &str, home: &Path) -> io::Result<PathBuf> {
    let expanded = match value.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with(SEPARATORS) => {
            home.join(rest.trim_start_matches(SEPARATORS))
        }
        _ => PathBuf::from(value),
    };
    if value.is_empty() || !expanded.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("--config-dir {value:?} must be an absolute path (or start with ~)"),
        ));
    }
    Ok(expanded)
}

/// Resolves an agent's skills folder, honoring its config directory variable.
pub(crate) fn skills_root(
    agent: &str,
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<PathBuf> {
    Ok(agent_home(agent, home, var)?.join("skills"))
}

/// An agent's own folder, such as `~/.claude`, honoring its config directory variable.
pub(crate) fn agent_home(
    agent: &str,
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<PathBuf> {
    let config_dir = |name: &str, default: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(default))
    };
    match agent {
        "codex" => Ok(config_dir("CODEX_HOME", ".codex")),
        "claude" => Ok(config_dir("CLAUDE_CONFIG_DIR", ".claude")),
        "copilot" => Ok(config_dir("COPILOT_HOME", ".copilot")),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown agent {agent:?}; use codex, claude, or copilot"),
        )),
    }
}

pub(crate) fn home_dir() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = env::var_os("HOME");
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot find your home directory"))
}

/// The files `install` writes, as paths such as `factory/SKILL.md`, relative to the skills folder.
pub(crate) fn bundled_skill_files() -> impl Iterator<Item = &'static str> {
    BUNDLED
        .iter()
        .map(|(path, _)| *path)
        .filter(|path| !path.starts_with("licenses/"))
}

/// The names of the skills `install` writes, in order.
pub(crate) fn bundled_skill_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = BUNDLED
        .iter()
        .filter_map(|(path, _)| path.split('/').next())
        .filter(|name| *name != "licenses")
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

fn install_to(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    for name in bundled_skill_names() {
        let dir = root.join(name);
        let staging = root.join(format!(".{name}.installing"));
        remove_if_exists(&staging)?;
        fs::create_dir(&staging)?;
        if let Err(error) = write_skill(name, &staging).and_then(|()| replace_dir(&staging, &dir)) {
            let _ = fs::remove_dir_all(&staging);
            return Err(io::Error::new(
                error.kind(),
                format!("cannot install skill {}: {error}", dir.display()),
            ));
        }
        println!("Installed: {}", dir.display());
    }
    for name in LEGACY_SKILLS {
        let dir = root.join(name);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("cannot remove legacy skill {}: {error}", dir.display()),
                )
            })?;
            println!("Removed: {}", dir.display());
        }
    }
    remove_retired_humanizer(root);
    Ok(())
}

/// The files the factory installed for Humanizer before it dropped the skill, kept only to
/// recognise that copy. They are compared against, never installed.
const RETIRED_HUMANIZER_FILES: [(&str, &[u8]); 3] = [
    ("SKILL.md", include_bytes!("retired/humanizer-SKILL.md")),
    (
        "agents/openai.yaml",
        include_bytes!("retired/humanizer-openai.yaml"),
    ),
    ("LICENSE", include_bytes!("retired/humanizer-LICENSE")),
];

/// Removes a `humanizer` skill folder the factory installed earlier. Only an exact, unmodified
/// copy goes: the same files and nothing else, none of them a symlink. Anything else is the
/// user's own and stays. A failure to remove it is a warning, not an install failure.
fn remove_retired_humanizer(root: &Path) {
    let dir = root.join("humanizer");
    if !is_retired_humanizer(&dir) {
        return;
    }
    match fs::remove_dir_all(&dir) {
        Ok(()) => println!("Removed: {}", dir.display()),
        Err(error) => eprintln!(
            "warning: cannot remove retired skill {}: {error}",
            dir.display()
        ),
    }
}

fn is_retired_humanizer(dir: &Path) -> bool {
    let mut found = Vec::new();
    if !collect_files(dir, Path::new(""), &mut found)
        || found.len() != RETIRED_HUMANIZER_FILES.len()
    {
        return false;
    }
    // Git may have checked the files out with CRLF line endings, so compare without them.
    let plain = |text: &[u8]| {
        text.iter()
            .copied()
            .filter(|byte| *byte != b'\r')
            .collect::<Vec<_>>()
    };
    RETIRED_HUMANIZER_FILES.iter().all(|(path, content)| {
        found.iter().any(|found| found == Path::new(path))
            && fs::read(dir.join(path)).is_ok_and(|actual| plain(&actual) == plain(content))
    })
}

/// Lists the files under `dir` relative to it. Returns false when `dir` is missing, or when it
/// holds anything but plain files and folders, such as a symlink.
fn collect_files(dir: &Path, relative: &Path, found: &mut Vec<PathBuf>) -> bool {
    let Ok(kind) = dir.symlink_metadata().map(|meta| meta.file_type()) else {
        return false;
    };
    if !kind.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries {
        let Ok(entry) = entry else { return false };
        let Ok(kind) = entry.file_type() else {
            return false;
        };
        let path = relative.join(entry.file_name());
        if kind.is_file() {
            found.push(path);
        } else if !(kind.is_dir() && collect_files(&entry.path(), &path, found)) {
            return false;
        }
    }
    true
}

/// Replaces `dir` with `staging`, so the skill matches the bundle exactly.
fn replace_dir(staging: &Path, dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        return fs::rename(staging, dir);
    }
    let previous = dir.with_file_name(format!(
        ".{}.previous",
        dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    remove_if_exists(&previous)?;
    fs::rename(dir, &previous)?;
    if let Err(error) = fs::rename(staging, dir) {
        let _ = fs::rename(&previous, dir);
        return Err(error);
    }
    fs::remove_dir_all(previous)
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_skill(name: &str, dir: &Path) -> io::Result<()> {
    for (path, content) in BUNDLED
        .iter()
        .filter(|(path, _)| path.split('/').next() == Some(name))
    {
        let relative = path.split_once('/').unwrap().1;
        let destination = dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, content)?;
    }
    let license = match name {
        "security-review" => "licenses/awesome-copilot-LICENSE",
        _ if LOCAL_SKILLS.contains(&name) => "",
        _ => "licenses/mattpocock-skills-LICENSE",
    };
    if let Some((_, content)) = BUNDLED.iter().find(|(path, _)| *path == license) {
        fs::write(dir.join("LICENSE"), content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_root_honors_agent_config_directories() {
        let home = Path::new("/home/user");
        let unset = |_: &str| None;
        let set = |name: &str| Some(OsString::from(format!("/custom/{name}")));
        let empty = |_: &str| Some(OsString::new());
        let root =
            |agent, var: &dyn Fn(&str) -> Option<OsString>| skills_root(agent, home, var).unwrap();
        assert_eq!(root("claude", &unset), home.join(".claude/skills"));
        assert_eq!(root("claude", &empty), home.join(".claude/skills"));
        assert_eq!(
            root("claude", &set),
            Path::new("/custom/CLAUDE_CONFIG_DIR/skills")
        );
        assert_eq!(root("codex", &unset), home.join(".codex/skills"));
        assert_eq!(root("codex", &set), Path::new("/custom/CODEX_HOME/skills"));
        assert_eq!(root("copilot", &unset), home.join(".copilot/skills"));
        assert_eq!(root("copilot", &empty), home.join(".copilot/skills"));
        assert_eq!(
            root("copilot", &set),
            Path::new("/custom/COPILOT_HOME/skills")
        );
        assert!(skills_root("other", home, unset).is_err());
    }
    /// Names that belong to this repository, not to the projects the skills are installed in.
    /// Concrete commands and branches go in AGENTS.md, README or .github/instructions instead.
    const PROJECT_SPECIFIC: [&str; 3] = ["verify.sh", "tds-factory", "for this project"];
    /// Workflow skills must also not assume this repository's branch name.
    const WORKFLOW_SKILLS: [&str; 3] = ["factory-full", "factory-balanced", "factory-fast"];

    fn bundled_text(path: &str) -> String {
        BUNDLED
            .iter()
            .find(|(bundled, _)| *bundled == path)
            .map(|(_, content)| String::from_utf8_lossy(content).into_owned())
            .unwrap_or_else(|| panic!("{path} is not bundled"))
    }

    #[test]
    fn selector_and_execution_workflows_are_bundled() {
        let selector = bundled_text("factory/SKILL.md");
        assert_eq!(selector.lines().nth(1), Some("name: factory"));
        for skill in WORKFLOW_SKILLS {
            let workflow = bundled_text(&format!("{skill}/SKILL.md"));
            assert_eq!(
                workflow.lines().nth(1),
                Some(format!("name: {skill}").as_str())
            );
            assert!(selector.contains(&format!("../{skill}/SKILL.md")));
        }
    }

    #[test]
    fn shared_procedures_record_run_context_with_options_the_cli_accepts_and_continue_safely() {
        let text = bundled_text("factory-procedures/SKILL.md");
        let context = text
            .split_once("- **Context:**")
            .and_then(|(_, rest)| rest.split_once("- **Done:**"))
            .expect("the shared procedures have no Context step")
            .0;
        let mut flags: Vec<&str> = context
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|word| word.starts_with("--") && word.len() > 2)
            .collect();
        flags.sort_unstable();
        flags.dedup();
        // The procedures name every option of the context; each one is a real `run update` option.
        for flag in [
            "--mode",
            "--worktree",
            "--branch",
            "--base-commit",
            "--spec",
            "--ticket",
            "--ticket-status",
            "--summary",
            "--pr",
            "--blocking-reason",
        ] {
            assert!(flags.contains(&flag), "the context does not record {flag}");
        }
        for flag in flags.iter().filter(|flag| !flag.starts_with("--clear-<")) {
            assert!(
                crate::run::UPDATE_HELP.contains(flag),
                "the context names {flag}, which `run update` does not accept"
            );
        }
        // A plan kept only in the conversation is saved and recorded with the existing --spec.
        let saving = text
            .split_once("## Saving the plan")
            .expect("the shared procedures cannot save a plan")
            .1
            .split("\n## ")
            .next()
            .unwrap();
        for needle in [
            "--spec",
            "git rev-parse --absolute-git-dir",
            "acceptance criteria",
        ] {
            assert!(saving.contains(needle), "saving the plan lacks {needle}");
        }
        // Continuing: inspect first, verify the recorded state, and never discard work.
        let continuing = text
            .split_once("## Continuing a run")
            .expect("the shared procedures have no continuation protocol")
            .1;
        let continuing = continuing.split("\n## ").next().unwrap();
        for needle in [
            "factory run inspect",
            "factory run check",
            "recorded branch",
            "ancestor of `HEAD`",
            "unexpected changes",
            "Never reset, clean",
        ] {
            assert!(continuing.contains(needle), "continuation lacks {needle}");
        }
    }

    #[test]
    fn workflow_skills_use_the_shared_procedures_and_report_their_own_mode() {
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md"));
            assert!(
                text.contains("(../factory-procedures/SKILL.md)"),
                "{skill} does not link the shared procedures"
            );
            for section in [
                "Run reporting",
                "Continuing a run",
                "Workspace",
                "Tracker setup",
                "Pull request and CI",
            ] {
                assert!(
                    text.contains(&format!("{section} in [factory-procedures]")),
                    "{skill} does not point to {section}"
                );
            }
            assert!(
                text.contains("Verification in [factory-procedures]"),
                "{skill} lacks Verification"
            );
            if skill != "factory-fast" {
                for section in ["Committing for Check", "Reworks"] {
                    assert!(
                        text.contains(&format!("{section} in [factory-procedures]")),
                        "{skill} lacks {section}"
                    );
                }
            }
            assert!(
                !text.contains("Waiting for a decision"),
                "{skill} still reports a decision wait as running"
            );
            assert_eq!(
                text.contains("Saving the plan in [factory-procedures]"),
                skill != "factory-full",
                "{skill} saves a conversational plan only in the lighter workflows"
            );
            assert!(
                text.contains(&format!("`--mode {skill}`")),
                "{skill} reports its own mode"
            );
        }
    }

    #[test]
    fn shared_procedures_cover_check_and_rework() {
        let text = bundled_text("factory-procedures/SKILL.md");
        for needle in [
            "## Verification",
            "## Committing for Check",
            "## Reworks",
            "--rework",
            "Decision needed on remaining Check findings",
            "Review pinned at <hash>",
        ] {
            assert!(text.contains(needle), "shared procedures lack {needle}");
        }
    }

    /// Whether a bundled skill's frontmatter keeps it from being invoked by the model.
    fn is_user_only(skill: &str) -> bool {
        let text = bundled_text(&format!("{skill}/SKILL.md")).replace("\r\n", "\n");
        let frontmatter = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(frontmatter, _)| frontmatter)
            .unwrap_or_else(|| panic!("{skill} has no frontmatter"));
        frontmatter
            .lines()
            .any(|line| line.trim() == "disable-model-invocation: true")
    }

    #[test]
    fn shared_procedures_are_not_invoked_on_their_own() {
        assert!(is_user_only("factory-procedures"));
        let codex = bundled_text("factory-procedures/agents/openai.yaml");
        assert!(
            codex.contains("allow_implicit_invocation: false"),
            "{codex}"
        );
    }

    #[test]
    fn bundled_procedures_explain_ticket_reports() {
        let procedures = bundled_text("factory-procedures/SKILL.md");
        assert!(procedures.contains("--ticket-report \"<reference>\" skill"));
        assert!(procedures.contains("--ticket-report \"<reference>\" activity"));
        assert!(procedures.contains("may be incomplete"));
    }

    #[test]
    fn workflows_read_the_user_only_skills_they_use() {
        let user_only: Vec<&str> = bundled_skill_names()
            .into_iter()
            .filter(|skill| is_user_only(skill))
            .collect();
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md")).replace("\r\n", "\n");
            let (preamble, body) = text
                .split_once("\n## ")
                .unwrap_or_else(|| panic!("{skill} has no sections"));
            let used: Vec<&str> = user_only
                .iter()
                .copied()
                .filter(|name| body.contains(&format!("`{name}`")))
                .collect();
            assert!(!used.is_empty(), "{skill} uses no user-only skill");
            let named: Vec<&str> = user_only
                .iter()
                .copied()
                .filter(|name| preamble.contains(&format!("`{name}`")))
                .collect();
            assert_eq!(
                named, used,
                "{skill} must name exactly the user-only skills it uses"
            );
            assert!(
                preamble
                    .contains("read their `SKILL.md` from that location instead of invoking them"),
                "{skill} does not say how to load its user-only skills"
            );
        }
    }

    #[test]
    fn bundled_skills_do_not_mention_humanizer() {
        for (path, content) in BUNDLED {
            if path.starts_with("licenses/") {
                continue;
            }
            let text = String::from_utf8_lossy(content).to_lowercase();
            assert!(!text.contains("humanizer"), "{path} mentions humanizer");
        }
        let procedures = bundled_text("factory-procedures/SKILL.md");
        assert!(!procedures.contains("## Writing"));
        for skill in WORKFLOW_SKILLS {
            assert!(
                !bundled_text(&format!("{skill}/SKILL.md")).contains("## Writing"),
                "{skill} still has a Writing section"
            );
        }
    }

    #[test]
    fn the_procedures_keep_the_pull_request_current_without_rewriting_history() {
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        let section = procedures
            .split_once("## Pull request and CI")
            .map(|(_, rest)| rest)
            .expect("the procedures have a Pull request and CI section");
        for needle in [
            "factory pr sync <run-id> --base <base>",
            "gh pr view --json mergeStateStatus",
            "`BEHIND`",
            "`DIRTY`",
            "`UNKNOWN`",
            "normal merge commit",
            "never rebase, amend or force-push",
            "neither a Rework nor a CI fix",
            "not behind its base",
        ] {
            assert!(section.contains(needle), "missing: {needle}");
        }
    }

    #[test]
    fn the_procedures_ask_for_an_upgrade_instead_of_working_around_an_outdated_factory() {
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        // The skills ship with the binary, so a command it lacks means the two are out of step.
        for gone in [
            "unknown command",
            "do it by hand",
            "checks by hand",
            "watch the checks yourself",
            "git worktree add",
            "gh pr create",
            "gh pr checks",
        ] {
            assert!(
                !procedures.contains(gone),
                "the procedures still work around an outdated factory: {gone}"
            );
        }
        for needle in [
            "needs upgrading with `factory install`",
            "mark the run blocked with that reason",
        ] {
            assert!(procedures.contains(needle), "missing: {needle}");
        }
        // Every outcome of the commands the fallbacks used to replace is still handled.
        for outcome in [
            "passed",
            "failed",
            "cancelled",
            "timeout",
            "head-changed",
            "no-ci",
            "not-open",
            "current",
            "pushed",
            "needs-verification",
            "conflict",
        ] {
            assert!(
                procedures.contains(&format!("- `{outcome}` (")),
                "the procedures lose the {outcome} outcome"
            );
        }
        // Both commands still explain their exit status 1.
        assert_eq!(
            procedures.matches("Exit status 1 is a problem").count(),
            2,
            "pr wait and pr sync both explain exit status 1"
        );
        // So does a host without factory support.
        assert!(procedures.contains("Watch every check there until all have completed"));
        // The hard rules say why.
        for reason in [
            "the user may be working in it",
            "The review is pinned to a commit hash",
            "merging is the human's review decision",
        ] {
            assert!(procedures.contains(reason), "missing reason: {reason}");
        }
    }

    #[test]
    fn the_procedures_start_a_run_naming_the_agent_with_an_option_the_cli_accepts() {
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        let start = procedures
            .split_once("- **Start:**")
            .and_then(|(_, rest)| rest.split_once("\n- **Stage:**"))
            .map(|(start, _)| start)
            .expect("the procedures have a Start step");
        assert!(
            start.contains("factory run start \"<short task title>\" --agent <agent>"),
            "{start}"
        );
        // The three values it names are the ones the command takes.
        for (word, host) in [
            ("claude", crate::routing::Host::Claude),
            ("codex", crate::routing::Host::Codex),
            ("copilot", crate::routing::Host::Copilot),
        ] {
            assert!(
                start.contains(&format!("`{word}`")),
                "Start does not name {word}"
            );
            assert_eq!(
                crate::routing::parse_start_options(&["--agent", word]),
                Ok(Some(host))
            );
        }
        // A policy refusal is passed on to the user, and an older command that lacks the option is reported.
        assert!(
            start.contains("tell the user what it says and stop"),
            "{start}"
        );
        assert!(start.contains("including `--agent`"), "{start}");
    }

    #[test]
    fn the_model_routing_procedure_says_how_to_read_and_apply_the_selection() {
        // A Windows checkout may have CRLF line endings.
        // Only Check uses it, so it lives in a reference file read when Check starts.
        let section =
            bundled_text("factory-procedures/references/model-routing.md").replace("\r\n", "\n");
        assert!(
            bundled_text("factory-procedures/SKILL.md")
                .contains("[references/model-routing.md](references/model-routing.md)"),
            "the procedures do not point to the model routing reference"
        );
        for needle in [
            "factory run inspect <id> --json",
            "`model_routing`",
            "`overrides.check`",
            "`tasks.check`",
            "never a fork",
            "only the aliases `opus`, `sonnet`, `haiku` and `fable`",
            "model routing not applied",
            "Copilot CLI",
            "do not claim a switch",
            "only the entry of the agent named with `--agent`",
            "never another agent's",
        ] {
            assert!(section.contains(needle), "Model routing lacks {needle}");
        }
        let review = bundled_text("code-review/SKILL.md");
        for needle in ["model and an effort for the review agents", "not forks"] {
            assert!(review.contains(needle), "code-review lacks {needle}");
        }
    }

    #[test]
    fn only_the_workflows_with_a_check_point_to_the_model_routing_procedure() {
        for skill in ["factory-full", "factory-balanced"] {
            let text = bundled_text(&format!("{skill}/SKILL.md"));
            assert!(
                text.contains("(../factory-procedures/references/model-routing.md)")
                    && text.contains("at the start of every Check pass, including after a Rework"),
                "{skill} does not read Model routing at every Check pass"
            );
        }
        let fast = bundled_text("factory-fast/SKILL.md");
        assert!(
            !fast.contains("Model routing"),
            "factory-fast has no Check to route"
        );
    }

    #[test]
    fn bundled_skills_do_not_name_this_repository() {
        let mut offenders = Vec::new();
        for (path, content) in BUNDLED {
            let (skill, _) = path.split_once('/').unwrap();
            if skill == "licenses" {
                continue;
            }
            let text = String::from_utf8_lossy(content);
            let lowercase = text.to_lowercase();
            for term in PROJECT_SPECIFIC {
                if lowercase.contains(term) {
                    offenders.push(format!("{path}: {term}"));
                }
            }
            let is_workflow = WORKFLOW_SKILLS.contains(&skill);
            if is_workflow
                && text
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|w| w == "develop")
            {
                offenders.push(format!("{path}: develop"));
            }
        }
        assert!(
            offenders.is_empty(),
            "bundled skills must not name this repository (see AGENTS.md): {offenders:?}"
        );
    }

    #[test]
    fn setup_links_exactly_the_bundled_issue_tracker_templates() {
        let setup = bundled_text("setup-matt-pocock-skills/SKILL.md");
        let mut linked: Vec<&str> = setup
            .split("](./")
            .skip(1)
            .filter_map(|rest| rest.split_once(')'))
            .map(|(target, _)| target)
            .filter(|target| target.starts_with("issue-tracker-"))
            .collect();
        linked.sort_unstable();
        linked.dedup();
        let mut bundled: Vec<&str> = bundled_skill_files()
            .filter_map(|path| path.strip_prefix("setup-matt-pocock-skills/"))
            .filter(|name| name.starts_with("issue-tracker-") && name.ends_with(".md"))
            .collect();
        bundled.sort_unstable();
        assert_eq!(linked, bundled);
        assert!(linked.contains(&"issue-tracker-azure-boards.md"));
    }

    #[test]
    fn setup_stops_on_missing_azure_boards_details_without_a_partial_tracker_file() {
        let setup = bundled_text("setup-matt-pocock-skills/SKILL.md");
        let (_, azure) = setup
            .split_once("An Azure Boards tracker needs")
            .expect("setup has an Azure Boards part");
        assert!(azure.contains("is missing, stop and tell the user"));
        assert!(azure.contains("partial `docs/agents/issue-tracker.md`"));
    }

    #[test]
    fn azure_boards_template_removes_tags_with_a_replace_the_cli_cannot_send() {
        // Checked against Azure DevOps: `az boards work-item update --fields System.Tags=...` adds to
        // the tags, and `show --fields` fails with the expanded output, so neither can remove a role.
        let template = bundled_text("setup-matt-pocock-skills/issue-tracker-azure-boards.md");
        let roles = template
            .split_once("- **Apply / remove triage roles**:")
            .and_then(|(_, rest)| rest.split_once("\n- **"))
            .expect("the template explains triage roles")
            .0;
        assert!(!roles.contains("replaces the whole list"));
        assert!(!roles.contains("--fields System.Tags"));
        assert!(roles.contains(r#"--query 'fields."System.Tags"'"#));
        assert!(roles.contains(r#""op":"replace","path":"/fields/System.Tags""#));
        assert!(roles.contains("Content-Type=application/json-patch+json"));
    }

    #[test]
    fn azure_boards_template_names_no_process_specific_types_or_states() {
        let template = bundled_text("setup-matt-pocock-skills/issue-tracker-azure-boards.md");
        let found: Vec<String> = [
            "User Story",
            "Product Backlog Item",
            "Active",
            "Resolved",
            "Done",
            "To Do",
            "Closed",
        ]
        .into_iter()
        .flat_map(|name| {
            [
                format!("`{name}`"),
                format!("\"{name}\""),
                format!("--state {name}"),
                format!("--type {name}"),
            ]
        })
        .filter(|used| template.contains(used))
        .collect();
        assert!(
            found.is_empty(),
            "template hardcodes process names as values: {found:?}"
        );
    }

    /// Returns why `arguments` (the text after `--route-parameters`) is not a
    /// list of separately double-quoted `"name=value"` pairs.
    fn route_parameter_problem(arguments: &str) -> Option<String> {
        let mut arguments = arguments.split_whitespace().peekable();
        if arguments.peek().is_none() {
            return Some("no arguments".to_string());
        }
        arguments.find_map(|argument| {
            let inner = argument
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'));
            match inner {
                Some(inner) if !inner.contains('"') && inner.matches('=').count() == 1 => None,
                _ => Some(format!("not exactly one quoted name=value: {argument}")),
            }
        })
    }

    #[test]
    fn route_parameter_checker_rejects_unquoted_and_merged_pairs() {
        assert_eq!(
            route_parameter_problem("\"project=<project>\" \"workItemId=<id>\""),
            None
        );
        assert!(route_parameter_problem("\"project=<project>\" type=<type>").is_some());
        assert!(route_parameter_problem("\"project=<project> type=<type>\"").is_some());
        assert!(route_parameter_problem("project=<project>").is_some());
    }

    #[test]
    fn azure_boards_route_parameters_are_quoted_pairs() {
        const FLAG: &str = "--route-parameters ";
        for path in [
            "setup-matt-pocock-skills/issue-tracker-azure-boards.md",
            "setup-matt-pocock-skills/SKILL.md",
        ] {
            let text = bundled_text(path);
            assert!(text.contains(FLAG), "{path} has no {FLAG}");
            for (index, _) in text.match_indices(FLAG) {
                let after = &text[index + FLAG.len()..];
                let end = [after.find(" --"), after.find(['`', '\n'])]
                    .into_iter()
                    .flatten()
                    .min()
                    .unwrap_or(after.len());
                let arguments = &after[..end];
                assert_eq!(
                    route_parameter_problem(arguments),
                    None,
                    "{path}: bad route parameters: {arguments}"
                );
            }
        }
    }

    #[test]
    fn skills_roots_resolves_every_listed_agent_once() {
        let home = Path::new("/home/user");
        let unset = |_: &str| None;
        assert_eq!(
            skills_roots("codex, claude,codex", &[], home, unset).unwrap(),
            [home.join(".codex/skills"), home.join(".claude/skills")]
        );
        assert_eq!(
            skills_roots("copilot", &[], home, unset).unwrap(),
            [home.join(".copilot/skills")]
        );
        for invalid in ["", "codex,", "codex,vscode", "all"] {
            assert!(
                skills_roots(invalid, &[], home, unset).is_err(),
                "{invalid:?}"
            );
        }
        let error = skills_roots("codex,vscode", &[], home, unset).unwrap_err();
        assert!(error.to_string().contains("vscode"));
    }
    fn dirs(flags: &[&str], var: Option<&str>, home: &Path) -> io::Result<Vec<PathBuf>> {
        let flags: Vec<String> = flags.iter().map(|flag| flag.to_string()).collect();
        claude_config_dirs(&flags, home, |name| {
            (name == "CLAUDE_CONFIG_DIR")
                .then(|| var.map(OsString::from))
                .flatten()
        })
    }

    fn text(path: PathBuf) -> String {
        path.display().to_string()
    }

    #[test]
    fn config_dir_flags_win_over_the_variable_and_the_default() {
        let home = test_abs("home/user");
        let env = text(test_abs("env"));
        assert_eq!(dirs(&[], None, &home).unwrap(), [home.join(".claude")]);
        assert_eq!(dirs(&[], Some(""), &home).unwrap(), [home.join(".claude")]);
        assert_eq!(dirs(&[], Some(&env), &home).unwrap(), [test_abs("env")]);
        let (a, b) = (text(test_abs("a")), text(test_abs("b")));
        assert_eq!(
            dirs(&[&a, &b], Some(&env), &home).unwrap(),
            [test_abs("a"), test_abs("b")]
        );
    }

    #[test]
    fn config_dirs_expand_the_tilde_and_drop_duplicates() {
        let home = test_abs("home/user");
        let same = text(home.join("x"));
        assert_eq!(
            dirs(&["~/x", "~", &same, "~/x/"], None, &home).unwrap(),
            [home.join("x"), home.clone()]
        );
        // Only a leading `~` of its own is the home folder.
        assert!(dirs(&["~other/x"], None, &home).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_backslash_follows_the_tilde_on_windows() {
        let home = test_abs("home/user");
        assert_eq!(
            dirs(&["~\\x\\y"], None, &home).unwrap(),
            [home.join("x").join("y")]
        );
    }

    #[test]
    fn relative_config_dirs_are_rejected() {
        let home = test_abs("home/user");
        let ok = text(test_abs("ok"));
        for bad in ["rel", "./rel", "", "../x"] {
            let error = dirs(&[&ok, bad], None, &home).unwrap_err();
            assert!(error.to_string().contains("absolute"), "{bad:?}");
        }
    }

    #[test]
    fn config_dirs_need_the_claude_agent() {
        let home = test_abs("home/user");
        let flags = [text(test_abs("a")), text(test_abs("b"))];
        assert!(skills_roots("codex", &flags, &home, |_: &str| None).is_err());
        assert_eq!(
            skills_roots("codex,claude", &flags, &home, |_: &str| None).unwrap(),
            [
                home.join(".codex/skills"),
                test_abs("a").join("skills"),
                test_abs("b").join("skills")
            ]
        );
    }

    type Mutation = fn(&Path);
    type RetiredFile = (&'static str, Vec<u8>);

    /// The files the factory installed for Humanizer, read from the copies kept for the comparison.
    fn retired_humanizer_files() -> Vec<RetiredFile> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/retired");
        [
            ("SKILL.md", "humanizer-SKILL.md"),
            ("agents/openai.yaml", "humanizer-openai.yaml"),
            ("LICENSE", "humanizer-LICENSE"),
        ]
        .map(|(installed, kept)| (installed, fs::read(dir.join(kept)).unwrap()))
        .into()
    }

    fn write_retired_humanizer(root: &Path, crlf: bool) {
        for (path, content) in retired_humanizer_files() {
            let content = if crlf {
                String::from_utf8(content)
                    .unwrap()
                    .replace("\r\n", "\n")
                    .replace('\n', "\r\n")
                    .into_bytes()
            } else {
                content
            };
            let file = root.join("humanizer").join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
    }

    /// Skills roots for every agent plus two `--config-dir` folders, as `factory install` resolves them.
    fn all_skills_roots(home: &Path) -> Vec<PathBuf> {
        let configs = [
            home.join("config-a").to_string_lossy().into_owned(),
            home.join("config-b").to_string_lossy().into_owned(),
        ];
        let roots = skills_roots("codex,claude,copilot", &configs, home, |_: &str| None).unwrap();
        assert_eq!(roots.len(), 4);
        roots
    }

    #[test]
    fn install_creates_no_humanizer_folder() {
        let home = tempfile::tempdir().unwrap();
        for root in all_skills_roots(home.path()) {
            install_to(&root).unwrap();
            assert!(!root.join("humanizer").exists(), "{root:?}");
        }
        assert!(!bundled_skill_names().contains(&"humanizer"));
        assert!(!bundled_skill_files().any(|path| path.starts_with("humanizer/")));
    }

    #[test]
    fn install_removes_the_humanizer_the_factory_installed_earlier() {
        let home = tempfile::tempdir().unwrap();
        for root in all_skills_roots(home.path()) {
            write_retired_humanizer(&root, false);
            install_to(&root).unwrap();
            assert!(!root.join("humanizer").exists(), "{root:?}");
            assert!(root.join("tdd/SKILL.md").is_file(), "{root:?}");
        }
    }

    #[test]
    fn install_removes_the_humanizer_when_git_changed_its_line_endings() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("skills");
        write_retired_humanizer(&root, true);
        install_to(&root).unwrap();
        assert!(!root.join("humanizer").exists());
    }

    #[test]
    fn install_keeps_a_humanizer_that_is_not_the_factory_copy() {
        let home = tempfile::tempdir().unwrap();
        let mutations: [(&str, Mutation); 5] = [
            ("edited skill", |h| {
                fs::write(h.join("SKILL.md"), "my own edits").unwrap()
            }),
            ("edited yaml", |h| {
                fs::write(h.join("agents/openai.yaml"), "mine").unwrap()
            }),
            ("extra file", |h| {
                fs::write(h.join("notes.md"), "mine").unwrap()
            }),
            ("other license", |h| {
                fs::write(h.join("LICENSE"), "someone else").unwrap()
            }),
            ("no license", |h| {
                fs::remove_file(h.join("LICENSE")).unwrap()
            }),
        ];
        for (name, mutate) in mutations {
            let root = home.path().join(name);
            write_retired_humanizer(&root, false);
            mutate(&root.join("humanizer"));
            install_to(&root).unwrap();
            assert!(root.join("humanizer").is_dir(), "{name}");
            assert!(root.join("tdd/SKILL.md").is_file(), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn install_keeps_a_humanizer_that_contains_a_symlink() {
        let home = tempfile::tempdir().unwrap();
        let outside = home.path().join("outside");
        let root = home.path().join("file-link");
        write_retired_humanizer(&root, false);
        let skill = root.join("humanizer/SKILL.md");
        fs::remove_file(&skill).unwrap();
        fs::write(&outside, &retired_humanizer_files()[0].1).unwrap();
        std::os::unix::fs::symlink(&outside, &skill).unwrap();
        install_to(&root).unwrap();
        assert!(root.join("humanizer").is_dir());

        let target = home.path().join("target");
        write_retired_humanizer(&target, false);
        let root = home.path().join("dir-link");
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(target.join("humanizer"), root.join("humanizer")).unwrap();
        install_to(&root).unwrap();
        assert!(root.join("humanizer").symlink_metadata().is_ok());
        assert!(target.join("humanizer/SKILL.md").is_file());
    }

    #[test]
    fn selector_replaces_full_and_all_entry_points_survive_reinstall_for_each_agent() {
        let home = tempfile::tempdir().unwrap();
        let roots = skills_roots("codex,claude,copilot", &[], home.path(), |_: &str| None).unwrap();
        for root in roots {
            fs::create_dir_all(root.join("factory")).unwrap();
            fs::write(
                root.join("factory/SKILL.md"),
                bundled_text("factory-full/SKILL.md"),
            )
            .unwrap();
            for _ in 0..2 {
                install_to(&root).unwrap();
                for skill in [
                    "factory",
                    "factory-full",
                    "factory-balanced",
                    "factory-fast",
                ] {
                    assert_eq!(
                        fs::read_to_string(root.join(skill).join("SKILL.md")).unwrap(),
                        bundled_text(&format!("{skill}/SKILL.md")),
                        "{root:?}: {skill}"
                    );
                }
            }
        }
    }

    #[test]
    fn installs_bundled_skills_and_replaces_existing_ones() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join("factory")).unwrap();
        fs::write(root.join("factory/SKILL.md"), "old content").unwrap();
        fs::write(root.join("factory/stale.md"), "removed upstream").unwrap();
        for legacy in [
            "factory2",
            "factory2-fast",
            "github-actions-hardening",
            "rust-best-practices",
        ] {
            fs::create_dir_all(root.join(legacy)).unwrap();
            fs::write(root.join(legacy).join("SKILL.md"), "legacy").unwrap();
        }
        fs::create_dir_all(root.join("my-skill")).unwrap();
        fs::write(root.join("my-skill/SKILL.md"), "user skill").unwrap();
        install_to(root).unwrap();
        assert!(fs::read_to_string(root.join("factory/SKILL.md"))
            .unwrap()
            .contains("name: factory"));
        assert!(!root.join("factory/stale.md").exists());
        assert!(!root.join("factory2").exists());
        assert!(!root.join("factory2-fast").exists());
        assert!(!root.join("github-actions-hardening").exists());
        assert!(!root.join("rust-best-practices").exists());
        assert_eq!(
            fs::read_to_string(root.join("my-skill/SKILL.md")).unwrap(),
            "user skill"
        );
        assert!(root
            .join("security-review/references/language-patterns.md")
            .is_file());
        assert!(root.join("security-review/LICENSE").is_file());
        assert!(root.join("setup-matt-pocock-skills/SKILL.md").is_file());
        assert!(root
            .join("setup-matt-pocock-skills/issue-tracker-github.md")
            .is_file());
        assert!(root
            .join("setup-matt-pocock-skills/issue-tracker-azure-boards.md")
            .is_file());
        assert!(root.join("setup-matt-pocock-skills/LICENSE").is_file());
        assert!(root.join("to-spec/SKILL.md").is_file());
        assert!(root.join("factory-full/SKILL.md").is_file());
        assert!(!root.join("factory-full/LICENSE").exists());
        assert!(root.join("factory-fast/SKILL.md").is_file());
        assert!(root.join("implement-spec-fast/SKILL.md").is_file());
        assert!(root.join("tdd/mocking.md").is_file());
        assert!(!root.join("factory-fast/LICENSE").exists());
        assert!(root.join("factory-balanced/SKILL.md").is_file());
        assert!(root.join("implement-spec-balanced/SKILL.md").is_file());
        assert!(!root.join("factory-balanced/LICENSE").exists());
        assert!(root.join("factory-procedures/SKILL.md").is_file());
        assert!(root
            .join("factory-procedures/references/model-routing.md")
            .is_file());
        assert!(!root.join("factory-procedures/LICENSE").exists());
        assert!(!root.join("implement-spec-balanced/LICENSE").exists());
        assert!(!root.join("implement-spec-fast/LICENSE").exists());
        assert!(root.join("triage/AGENT-BRIEF.md").is_file());
        assert!(root.join("triage/LICENSE").is_file());
        install_to(root).unwrap();
        let leftovers: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "leftover staging dirs: {leftovers:?}");
    }
}
