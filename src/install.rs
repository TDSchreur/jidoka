use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));

/// Skill directories from earlier releases that are no longer bundled: the pre-rename
/// `factory2` skills, development-only skills and the generic-name skills of earlier releases.
const LEGACY_SKILLS: [&str; 20] = [
    "factory2",
    "factory2-fast",
    "github-actions-hardening",
    "rust-best-practices",
    "setup-matt-pocock-skills",
    "grill-with-docs",
    "grilling",
    "domain-modeling",
    "codebase-design",
    "to-spec",
    "to-tickets",
    "implement-spec",
    "implement-spec-fast",
    "implement-spec-balanced",
    "tdd",
    "code-review",
    "security-review",
    "pr",
    "setup-tds-factory",
    "triage",
];

/// Skills maintained in this project, installed without a third-party license.
const LOCAL_SKILLS: [&str; 7] = [
    "factory",
    "factory-full",
    "factory-procedures",
    "factory-fast",
    "factory-balanced",
    "factory-implement-fast",
    "factory-implement-balanced",
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

/// Create the initial routing policy. Only a file in the old format is replaced; any other existing
/// configuration is kept.
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
        crate::routing::RoutingAgent::Copilot,
    ];
    use crate::routing::Starter;
    let created = crate::routing::install_starter(&path, &agents).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot create model routing policy {}: {error}",
                path.display()
            ),
        )
    })?;
    let shown = path.display().to_string();
    match created {
        Starter::Created => println!("{}", Starter::created_message(&shown, None)),
        Starter::Replaced(off) => println!("{}", Starter::replaced_message(&shown, off, None)),
        Starter::Left => {}
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
            "warning: cannot remove retired skill {}: {error}. It may be partly deleted and will not be retried, so remove the folder by hand.",
            dir.display()
        ),
    }
}

/// True when `dir` holds exactly the retired files and the folders they sit in, with the same
/// content (ignoring CR), and nothing else.
fn is_retired_humanizer(dir: &Path) -> bool {
    let mut found = Vec::new();
    if !collect_entries(dir, Path::new(""), &mut found) {
        return false;
    }
    let mut expected: Vec<PathBuf> = Vec::new();
    for (path, _) in RETIRED_HUMANIZER_FILES {
        let path = Path::new(path);
        expected.extend(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(Path::to_path_buf),
        );
        expected.push(path.to_path_buf());
    }
    expected.sort();
    found.sort();
    if found != expected {
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
        fs::read(dir.join(path)).is_ok_and(|actual| plain(&actual) == plain(content))
    })
}

/// Lists the files and folders under `dir` relative to it. Returns false when `dir` is missing, or when it
/// holds anything but plain files and folders, such as a symlink.
fn collect_entries(dir: &Path, relative: &Path, found: &mut Vec<PathBuf>) -> bool {
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
        found.push(path.clone());
        if !(kind.is_file() || kind.is_dir() && collect_entries(&entry.path(), &path, found)) {
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
        "factory-security-review" => "licenses/awesome-copilot-LICENSE",
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
    fn bundled_skill_links_to_other_skills_resolve() {
        let files: Vec<&str> = bundled_skill_files().collect();
        for file in files.iter().filter(|file| file.ends_with(".md")) {
            let text = bundled_text(file);
            let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir);
            for part in text.split("](").skip(1) {
                let Some(target) = part.split(')').next() else {
                    continue;
                };
                let target = target.split('#').next().unwrap_or("");
                if !target.starts_with("../") {
                    continue;
                }
                let mut segments: Vec<&str> = dir.split('/').collect();
                for segment in target.split('/') {
                    match segment {
                        ".." => {
                            segments.pop();
                        }
                        "." | "" => {}
                        other => segments.push(other),
                    }
                }
                let resolved = segments.join("/");
                assert!(
                    files.contains(&resolved.as_str()),
                    "{file} links to missing {target}"
                );
            }
        }
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
    fn finish_reports_tell_the_user_to_clean_up_the_run_after_merge() {
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        assert!(
            procedures.contains("`factory run cleanup <id>`"),
            "the shared procedures do not mention factory run cleanup"
        );
        assert!(
            !procedures.contains("git worktree remove"),
            "the shared procedures still send the user to git worktree remove"
        );
        let finish = procedures
            .split_once("\n## Finish\n")
            .expect("the shared procedures have no Finish")
            .1
            .split("\n## ")
            .next()
            .unwrap();
        assert!(
            finish.contains("`factory run cleanup <id>`"),
            "the shared Finish does not mention factory run cleanup"
        );
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md"));
            let workflow_finish = text
                .split_once("## Finish")
                .unwrap_or_else(|| panic!("{skill} has no Finish"))
                .1;
            assert!(
                workflow_finish.contains("Finish in [factory-procedures]"),
                "{skill} Finish does not point to the shared Finish"
            );
        }
    }

    #[test]
    fn workflow_skills_use_the_shared_procedures_and_report_their_own_mode() {
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md")).replace("\r\n", "\n");
            assert!(
                text.contains("(../factory-procedures/SKILL.md)"),
                "{skill} does not link the shared procedures"
            );
            let shared = text
                .split_once("## Shared procedures\n")
                .unwrap_or_else(|| panic!("{skill} has no Shared procedures"))
                .1
                .split("\n## ")
                .next()
                .unwrap();
            assert!(
                shared.contains(
                    "Run reporting, Continuing a run (to continue an existing run) and Workspace in [factory-procedures]"
                ),
                "{skill} does not point to Run reporting, Continuing a run and Workspace"
            );
            for section in ["Tracker setup", "Define rules", "Finish"] {
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
    fn shared_procedures_say_how_workflows_use_the_bundled_skills() {
        let procedures = bundled_text("factory-procedures/SKILL.md");
        for needle in [
            "## Using the bundled skills",
            "they are siblings of this directory",
            "`factory-grill-with-docs`, `factory-to-spec` and `factory-to-tickets` can only be invoked by the user",
            "A workflow's instructions take precedence where they differ from those skills",
            "## Define rules",
            "Do not require extra approval of test seams or a separate ticket-breakdown approval",
        ] {
            assert!(procedures.contains(needle), "procedures lack {needle}");
        }
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md"));
            assert!(
                text.contains("Using the bundled skills in the shared procedures"),
                "{skill} does not point to Using the bundled skills"
            );
        }
        assert!(bundled_text("factory-full/SKILL.md")
            .contains("`factory-implement` can also only be invoked by the user"));
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
    fn bundled_skills_keep_the_run_in_the_starting_session() {
        let procedures = bundled_text("factory-procedures/SKILL.md");
        for phrase in [
            "## Run ownership",
            "never the whole run or several stages to one agent",
            "even when other instructions say to delegate",
        ] {
            assert!(procedures.contains(phrase), "procedures: {phrase}");
        }
        for path in [
            "factory/SKILL.md",
            "factory-full/SKILL.md",
            "factory-balanced/SKILL.md",
            "factory-fast/SKILL.md",
        ] {
            assert!(bundled_text(path).contains("Run ownership"), "{path}");
        }
    }

    #[test]
    fn bundled_skills_carry_the_skill_check() {
        for (path, wording) in [
            (
                "factory-procedures/SKILL.md",
                &["Skill check:", "wording only"][..],
            ),
            (
                "factory-code-review/SKILL.md",
                &["Skill check:", "no commit carries a `Skill check:`"][..],
            ),
            ("factory-pr/SKILL.md", &["**Skill check:**"][..]),
            (
                "factory-procedures/references/model-routing.md",
                &["Changing agent skills"][..],
            ),
        ] {
            let text = bundled_text(path);
            for phrase in wording {
                assert!(text.contains(phrase), "{path}: {phrase}");
            }
        }
    }

    #[test]
    fn workflows_read_the_user_only_skills_they_use() {
        let user_only: Vec<&str> = bundled_skill_names()
            .into_iter()
            .filter(|skill| is_user_only(skill))
            .collect();
        // The shared procedures name the user-only skills every workflow uses and say how to
        // load them; a workflow names any further one it uses.
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        let shared = procedures
            .split_once("\n## Using the bundled skills\n")
            .expect("the shared procedures have no Using the bundled skills")
            .1
            .split("\n## ")
            .next()
            .unwrap();
        assert!(
            shared.contains("read their `SKILL.md` from that location instead of invoking them"),
            "the shared procedures do not say how to load user-only skills"
        );
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
                .filter(|name| {
                    let name = format!("`{name}`");
                    shared.contains(&name) || preamble.contains(&name)
                })
                .collect();
            assert_eq!(
                named, used,
                "{skill} and the shared procedures must name exactly the user-only skills it uses"
            );
            assert!(
                preamble.contains("Using the bundled skills in the shared procedures"),
                "{skill} does not point to how its user-only skills are loaded"
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
        let section = bundled_text("factory-procedures/references/pull-request-and-ci.md")
            .replace("\r\n", "\n");
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
        let skill = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        let reference = bundled_text("factory-procedures/references/pull-request-and-ci.md")
            .replace("\r\n", "\n");
        assert!(
            skill.contains("(references/pull-request-and-ci.md)"),
            "the procedures do not link the pull request and CI reference"
        );
        let procedures = format!("{skill}\n{reference}");
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
            reference.matches("Exit status 1 is a problem").count(),
            2,
            "pr wait and pr sync both explain exit status 1"
        );
        // So does a host without factory support.
        assert!(reference.contains("Watch every check there until all have completed"));
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
        let section =
            bundled_text("factory-procedures/references/model-routing.md").replace("\r\n", "\n");
        let procedures = bundled_text("factory-procedures/SKILL.md");
        assert!(
            procedures.contains("[references/model-routing.md](references/model-routing.md)")
                && procedures.contains("applies to every Stage"),
            "the procedures do not point to the model routing reference for every Stage"
        );
        for needle in [
            "factory run inspect <id> --json",
            "`model_routing`",
            "`overrides.<stage>`",
            "`stages.<stage>`",
            "never a fork",
            "only the aliases `opus`, `sonnet`, `haiku` and `fable`",
            "neither the model nor the effort",
            "model routing not applied",
            "Copilot CLI",
            "do not claim a switch",
            "only the entry of the agent named with `--agent`",
            "never another agent's",
            // Each Stage says what is handed to agents and what stays with the coordinator.
            "the interview (factory-grill-with-docs) stays with you",
            "each ticket runs in its own fresh agent on the build choice",
            "tell `factory-code-review` the check choice",
            "You run `factory pr open`, `factory pr wait` and `factory pr sync` yourself",
        ] {
            assert!(section.contains(needle), "Model routing lacks {needle}");
        }
        let review = bundled_text("factory-code-review/SKILL.md");
        for needle in ["model and an effort for the review agents", "not forks"] {
            assert!(
                review.contains(needle),
                "factory-code-review lacks {needle}"
            );
        }
    }

    #[test]
    fn model_routing_is_read_at_every_stage_the_workflow_routes() {
        let link = "[Model routing](../factory-procedures/references/model-routing.md)";
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        let rule = procedures
            .split_once("\n## Model routing\n")
            .expect("the shared procedures have no Model routing")
            .1
            .split("\n## ")
            .next()
            .unwrap();
        for needle in [
            "Read it at the start of each Stage the workflow has, and again at the start of every Rework and every Check pass after one",
            "factory-fast: it never routes Define and Finish, so it reads this only at the start of Build and does not read it for Define or Finish",
        ] {
            assert!(rule.contains(needle), "Model routing rule lacks {needle}");
        }
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md")).replace("\r\n", "\n");
            // A routed Build hands each ticket to a fresh agent, overriding the build skill's
            // "do not require subagents".
            assert!(
                text.contains("When Build is routed (see [Model routing]")
                    && text.contains("hand each ticket"),
                "{skill} does not hand routed Build tickets to agents"
            );
            // The rule lives in the procedures; a workflow names it only where it adds something.
            assert!(
                !text.contains("At the start of Define, read")
                    && !text.contains("At the start of Finish, read"),
                "{skill} repeats the Model routing rule for Define or Finish"
            );
        }
        for skill in ["factory-full", "factory-balanced"] {
            let text = bundled_text(&format!("{skill}/SKILL.md"));
            assert!(
                text.contains(&format!(
                    "Follow {link} for Check: it says which model and effort"
                )),
                "{skill} does not name Model routing for Check"
            );
        }
        let full = bundled_text("factory-full/SKILL.md");
        assert!(
            full.contains(&format!("Then read {link} and follow it for Define")),
            "factory-full reads Model routing after Tracker setup in Define"
        );
        let fast = bundled_text("factory-fast/SKILL.md");
        assert!(
            !fast.contains("for Check")
                && !fast.contains("Check pass")
                && !fast.contains("for Define"),
            "factory-fast has no Check to route and does not route Define"
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
        let setup = bundled_text("factory-setup/SKILL.md");
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
            .filter_map(|path| path.strip_prefix("factory-setup/"))
            .filter(|name| name.starts_with("issue-tracker-") && name.ends_with(".md"))
            .collect();
        bundled.sort_unstable();
        assert_eq!(linked, bundled);
        assert!(linked.contains(&"issue-tracker-azure-boards.md"));
    }

    #[test]
    fn setup_stops_on_missing_azure_boards_details_without_a_partial_tracker_file() {
        let setup = bundled_text("factory-setup/SKILL.md");
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
        let template = bundled_text("factory-setup/issue-tracker-azure-boards.md");
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
        let template = bundled_text("factory-setup/issue-tracker-azure-boards.md");
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
            "factory-setup/issue-tracker-azure-boards.md",
            "factory-setup/SKILL.md",
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
            assert!(root.join("factory-tdd/SKILL.md").is_file(), "{root:?}");
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
        let mutations: [(&str, Mutation); 6] = [
            ("edited skill", |h| {
                fs::write(h.join("SKILL.md"), "my own edits").unwrap()
            }),
            ("edited yaml", |h| {
                fs::write(h.join("agents/openai.yaml"), "mine").unwrap()
            }),
            ("extra empty dir", |h| {
                fs::create_dir(h.join("extra")).unwrap()
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
            assert!(root.join("factory-tdd/SKILL.md").is_file(), "{name}");
        }
    }

    // Creating symlinks on Windows needs a privilege that CI and users often lack, so this runs on Unix only.
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
        for legacy in LEGACY_SKILLS {
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
        for old in LEGACY_SKILLS {
            assert!(!root.join(old).exists(), "{old}");
        }
        let mut installed: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != "my-skill")
            .collect();
        installed.sort();
        assert_eq!(installed, bundled_skill_names());
        assert!(
            installed.iter().all(|name| name.starts_with("factory")),
            "{installed:?}"
        );
        assert_eq!(
            fs::read_to_string(root.join("my-skill/SKILL.md")).unwrap(),
            "user skill"
        );
        assert!(root
            .join("factory-security-review/references/language-patterns.md")
            .is_file());
        assert!(root.join("factory-security-review/LICENSE").is_file());
        assert!(root.join("factory-setup/SKILL.md").is_file());
        assert!(root.join("factory-setup/issue-tracker-github.md").is_file());
        assert!(root
            .join("factory-setup/issue-tracker-azure-boards.md")
            .is_file());
        assert!(root.join("factory-setup/LICENSE").is_file());
        assert!(root.join("factory-to-spec/SKILL.md").is_file());
        assert!(root.join("factory-full/SKILL.md").is_file());
        assert!(!root.join("factory-full/LICENSE").exists());
        assert!(root.join("factory-fast/SKILL.md").is_file());
        assert!(root.join("factory-implement-fast/SKILL.md").is_file());
        assert!(root.join("factory-tdd/mocking.md").is_file());
        assert!(!root.join("factory-fast/LICENSE").exists());
        assert!(root.join("factory-balanced/SKILL.md").is_file());
        assert!(root.join("factory-implement-balanced/SKILL.md").is_file());
        assert!(!root.join("factory-balanced/LICENSE").exists());
        assert!(root.join("factory-procedures/SKILL.md").is_file());
        assert!(root
            .join("factory-procedures/references/model-routing.md")
            .is_file());
        assert!(root
            .join("factory-procedures/references/pull-request-and-ci.md")
            .is_file());
        assert!(!root.join("factory-procedures/LICENSE").exists());
        assert!(!root.join("factory-implement-balanced/LICENSE").exists());
        assert!(!root.join("factory-implement-fast/LICENSE").exists());
        assert!(root.join("factory-triage/AGENT-BRIEF.md").is_file());
        assert!(root.join("factory-triage/LICENSE").is_file());
        install_to(root).unwrap();
        let leftovers: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "leftover staging dirs: {leftovers:?}");
    }
}
