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

/// Installs the bundled skills for each agent in a comma-separated list such as `codex,claude`.
pub fn install(agents: &str) -> io::Result<()> {
    let roots = skills_roots(agents, &home_dir()?, |name| env::var_os(name))?;
    for root in roots {
        install_to(&root)?;
    }
    Ok(())
}

/// Resolves the skills folder of every listed agent before anything is installed.
fn skills_roots(
    agents: &str,
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<Vec<PathBuf>> {
    let mut names: Vec<&str> = Vec::new();
    for name in agents.split(',').map(str::trim) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|name| skills_root(name, home, &var))
        .collect()
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
    Ok(())
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
        "humanizer" => "licenses/humanizer-LICENSE",
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
            "git worktree list",
            "git branch --show-current",
            "merge-base --is-ancestor",
            "git status --porcelain",
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
                "Writing",
                "Pull request and CI",
            ] {
                assert!(
                    text.contains(&format!("{section} in [factory-procedures]")),
                    "{skill} does not point to {section}"
                );
            }
            assert_eq!(
                text.contains("check blocked --blocking-reason \"Decision needed on remaining Check findings\""),
                skill != "factory-fast",
                "{skill} reports blocked while waiting for a decision on Check findings"
            );
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
            assert_eq!(
                text.contains("--rework"),
                skill != "factory-fast",
                "{skill} reports Reworks only when it has Check"
            );
        }
    }

    #[test]
    fn writing_is_routed_through_the_bundled_humanizer() {
        // The writing procedure and `pr` name the skill; it ships in the bundle they are read from.
        for path in ["factory-procedures/SKILL.md", "pr/SKILL.md"] {
            assert!(bundled_text(path).contains("`humanizer`"), "{path}");
        }
        assert!(bundled_text("humanizer/SKILL.md").contains("**Embedded mode.**"));
        assert!(bundled_text("humanizer/SKILL.md").contains("**File mode.**"));
        // Every workflow reads that procedure, which covers both modes and what must survive editing.
        let writing = bundled_text("factory-procedures/SKILL.md")
            .split_once("## Writing")
            .expect("the shared procedures have no Writing section")
            .1
            .split("\n## ")
            .next()
            .unwrap()
            .to_lowercase();
        for needle in [
            "embedded mode",
            "file mode",
            "evidence",
            "qualifications",
            "findings",
            "right before you write the first",
            "still gets the humanizer pass",
        ] {
            assert!(
                writing.contains(needle),
                "the writing procedure lacks {needle}"
            );
        }
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md"));
            assert!(
                text.contains("Writing in [factory-procedures]"),
                "{skill} does not point to the writing procedure"
            );
        }
        // In this checkout the same names resolve as skills/<name>/ directories.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(root.join("skills/humanizer/SKILL.md").is_file());
        let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(agents.contains("(skills/humanizer/SKILL.md)"));
    }

    #[test]
    fn the_procedures_keep_the_pull_request_current_without_rewriting_history() {
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        let section = procedures
            .split_once("## Pull request and CI")
            .map(|(_, rest)| rest)
            .expect("the procedures have a Pull request and CI section");
        for needle in [
            "git merge-base --is-ancestor origin/<base> HEAD",
            "gh pr view --json mergeStateStatus",
            "`BEHIND`",
            "`DIRTY`",
            "keep the file list",
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
        let procedures = bundled_text("factory-procedures/SKILL.md").replace("\r\n", "\n");
        let section = procedures
            .split_once("## Model routing\n")
            .map(|(_, rest)| {
                rest.split_once("\n## ")
                    .map_or(rest, |(section, _)| section)
            })
            .expect("the procedures have a Model routing section");
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
                text.contains(
                    "Model routing in [factory-procedures](../factory-procedures/SKILL.md)"
                ),
                "{skill} does not point to Model routing"
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
    fn skills_roots_resolves_every_listed_agent_once() {
        let home = Path::new("/home/user");
        let unset = |_: &str| None;
        assert_eq!(
            skills_roots("codex, claude,codex", home, unset).unwrap(),
            [home.join(".codex/skills"), home.join(".claude/skills")]
        );
        assert_eq!(
            skills_roots("copilot", home, unset).unwrap(),
            [home.join(".copilot/skills")]
        );
        for invalid in ["", "codex,", "codex,vscode", "all"] {
            assert!(skills_roots(invalid, home, unset).is_err(), "{invalid:?}");
        }
        let error = skills_roots("codex,vscode", home, unset).unwrap_err();
        assert!(error.to_string().contains("vscode"));
    }
    #[test]
    fn humanizer_is_installed_with_its_own_license_for_every_agent() {
        let home = tempfile::tempdir().unwrap();
        let unset = |_: &str| None;
        let roots = skills_roots("codex,claude,copilot", home.path(), unset).unwrap();
        assert_eq!(roots.len(), 3);
        let upstream = bundled_text("licenses/humanizer-LICENSE");
        for root in roots {
            // An existing, outdated installation is replaced, including its license.
            fs::create_dir_all(root.join("humanizer")).unwrap();
            fs::write(root.join("humanizer/SKILL.md"), "old").unwrap();
            fs::write(root.join("humanizer/LICENSE"), "wrong license").unwrap();
            install_to(&root).unwrap();
            let skill = fs::read_to_string(root.join("humanizer/SKILL.md")).unwrap();
            assert!(skill.contains("name: humanizer"), "{root:?}");
            let license = fs::read_to_string(root.join("humanizer/LICENSE")).unwrap();
            assert_eq!(license, upstream, "{root:?}");
            assert!(license.contains("Siqi Chen"));
            assert!(!license.contains("Matt Pocock"));
            assert!(!fs::read_to_string(root.join("tdd/LICENSE"))
                .unwrap()
                .contains("Siqi Chen"));
        }
    }

    #[test]
    fn selector_replaces_full_and_all_entry_points_survive_reinstall_for_each_agent() {
        let home = tempfile::tempdir().unwrap();
        let roots = skills_roots("codex,claude,copilot", home.path(), |_: &str| None).unwrap();
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
        assert!(!root.join("factory-procedures/LICENSE").exists());
        assert!(!root.join("implement-spec-balanced/LICENSE").exists());
        assert!(!root.join("implement-spec-fast/LICENSE").exists());
        assert!(root.join("triage/AGENT-BRIEF.md").is_file());
        assert!(root.join("triage/LICENSE").is_file());
        assert!(root.join("humanizer/agents/openai.yaml").is_file());
        install_to(root).unwrap();
        let leftovers: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "leftover staging dirs: {leftovers:?}");
    }
}
