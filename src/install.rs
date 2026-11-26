use crate::agent::Agent;
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));

/// Skills maintained in this project, installed without a third-party license.
const LOCAL_SKILLS: [&str; 13] = [
    "jidoka",
    "jidoka-full",
    "jidoka-procedures",
    "jidoka-fast",
    "jidoka-balanced",
    "jidoka-implement-fast",
    "jidoka-implement-balanced",
    "jidoka-run-report",
    "jidoka-awesome-copilot",
    "jidoka-setup-knowledge-repo",
    "jidoka-cross-repo-change",
    "jidoka-work-through-plan",
    "jidoka-pr-watch",
];

/// The error for an agent name that is not one of `Agent::ALL`.
pub(crate) fn unknown_agent(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("unknown agent {name:?}; use {}", Agent::list()),
    )
}

/// Turns an answer such as `1,claude`, `2 3` or `all` into agents, without duplicates. Numbers count
/// from 1 in the install menu order, `Agent::ALL`.
///
/// # Errors
///
/// Fails naming the first entry that is not a number, name or `all`, and on an empty answer.
pub(crate) fn parse_agent_choice(answer: &str) -> io::Result<Vec<Agent>> {
    let mut agents: Vec<Agent> = Vec::new();
    for entry in answer
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
    {
        let chosen: Vec<Agent> = match (entry, Agent::parse(entry)) {
            ("all", _) => Agent::ALL.to_vec(),
            (_, Some(agent)) => vec![agent],
            (number, None) => match number.parse::<usize>() {
                Ok(n) if (1..=Agent::ALL.len()).contains(&n) => vec![Agent::ALL[n - 1]],
                _ => return Err(unknown_agent(number)),
            },
        };
        for agent in chosen {
            if !agents.contains(&agent) {
                agents.push(agent);
            }
        }
    }
    if agents.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no agent given; use {}", Agent::list()),
        ));
    }
    Ok(agents)
}

/// Selects agents from `--agent` or `JIDOKA_AGENT`. With neither
/// agents source, asks on `output`/`input` when `terminal` is true and otherwise fails.
pub(crate) fn select(
    agent_flag: Option<&str>,
    var: impl Fn(&str) -> Option<OsString>,
    terminal: bool,
    input: &mut impl io::BufRead,
    output: &mut impl io::Write,
) -> io::Result<Vec<Agent>> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    let env_agent = var("JIDOKA_AGENT")
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty());
    match agent_flag.map(str::to_string).or(env_agent) {
        Some(list) => parse_agent_choice(&list),
        None if terminal => ask_agents(input, output),
        None => Err(invalid(
            "No terminal to ask which agents to install for. Use --agent or set JIDOKA_AGENT, for example --agent codex,claude."
                .to_string(),
        )),
    }
}

fn ask_agents(input: &mut impl io::BufRead, output: &mut impl io::Write) -> io::Result<Vec<Agent>> {
    loop {
        writeln!(output, "Install the Jidoka skills for which agents?")?;
        for (index, agent) in Agent::ALL.iter().enumerate() {
            writeln!(output, "  {}) {}", index + 1, agent.name())?;
        }
        write!(
            output,
            "Enter one or more numbers or names separated by commas, or \"all\": "
        )?;
        output.flush()?;
        let mut answer = String::new();
        if input.read_line(&mut answer)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "No answer given. Use --agent or set JIDOKA_AGENT.",
            ));
        }
        if let Ok(agents) = parse_agent_choice(&answer) {
            return Ok(agents);
        }
        writeln!(output, "Invalid choice. Try again.")?;
    }
}

/// `jidoka install [--agent <list>]`: installs the skills and the default
/// routing policy, then writes the install record.
pub(crate) fn run(options: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    use io::IsTerminal;
    let agent = match options {
        [] => None,
        ["--agent", agent] => Some(*agent),
        ["--agent", _, invalid, ..] | [invalid, ..] => {
            return Err(format!("Invalid option {invalid:?}.").into());
        }
    };
    let stdin = io::stdin();
    let selection = select(
        agent,
        |name| env::var_os(name),
        stdin.is_terminal(),
        &mut stdin.lock(),
        &mut io::stdout(),
    )?;
    install(&selection)?;
    Ok(())
}

/// Installs the bundled skills for the selected agents, the default policy and the install record.
pub(crate) fn install(agents: &[Agent]) -> io::Result<()> {
    let var = |name: &str| env::var_os(name);
    install_skills(agents, &home_dir()?, &var, &mut io::stdout())?;
    let path = crate::install_record::path_in(var).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot find Jidoka config directory for the install record",
        )
    })?;
    crate::install_record::write(
        &path,
        &crate::install_record::InstallRecord::current(
            agents
                .iter()
                .map(|agent| agent.name().to_string())
                .collect(),
        ),
    )
}

/// The skills and the default policy of an install, without the install record; policy messages go to `out`.
pub(crate) fn install_skills(
    agents: &[Agent],
    home: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
    out: &mut impl io::Write,
) -> io::Result<()> {
    install_skill_files(agents, home, var, out)?;
    ensure_default_policy(var, out)
}

/// Only the skills of an install: no policy, no record.
pub(crate) fn install_skill_files(
    agents: &[Agent],
    home: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
    out: &mut impl io::Write,
) -> io::Result<()> {
    for root in skills_roots(agents, home, var) {
        install_to_reporting(&root, out)?;
    }
    Ok(())
}

/// Replace the routing policy and report what happened on `out`.
fn ensure_default_policy(
    var: impl Fn(&str) -> Option<OsString>,
    out: &mut impl io::Write,
) -> io::Result<()> {
    let path = replace_default_policy(var)?;
    writeln!(out, "Updated model routing policy {}.", path.display())?;
    Ok(())
}

/// Replace the routing policy with defaults for all agents.
pub(crate) fn replace_default_policy(
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<PathBuf> {
    let path = crate::routing::policy_path_in(var).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot find Jidoka config directory for model routing",
        )
    })?;
    crate::routing::replace_starter(&path, &crate::routing::POLICY_ORDER).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot replace model routing policy {}: {error}",
                path.display()
            ),
        )
    })?;
    Ok(path)
}

/// The skills folder of every listed agent.
pub(crate) fn skills_roots(
    agents: &[Agent],
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> Vec<PathBuf> {
    agents
        .iter()
        .map(|agent| skills_root(*agent, home, &var))
        .collect()
}

/// An agent's skills folder, inside its Agent home.
pub(crate) fn skills_root(
    agent: Agent,
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> PathBuf {
    agent.home(home, var).join("skills")
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

pub(crate) fn bundled_skill_text(path: &str) -> Option<&'static str> {
    BUNDLED
        .iter()
        .find(|(name, _)| *name == path)
        .and_then(|(_, bytes)| std::str::from_utf8(bytes).ok())
}

/// The files `install` writes, relative to the skills folder.
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

#[cfg(test)]
fn install_to(root: &Path) -> io::Result<()> {
    install_to_reporting(root, &mut io::stdout())
}

/// Installs the bundled skills into `root`, reporting each change on `out`.
fn install_to_reporting(root: &Path, out: &mut impl io::Write) -> io::Result<()> {
    fs::create_dir_all(root)?;
    for name in bundled_skill_names() {
        let dir = root.join(name);
        let staging = root.join(format!(".{name}.installing"));
        remove_if_exists(&staging)?;
        fs::create_dir(&staging)?;
        if let Err(error) = write_skill(name, &staging).and_then(|()| replace_dir(&staging, &dir)) {
            // Best-effort cleanup; the install error is the one returned.
            let _ = fs::remove_dir_all(&staging);
            return Err(io::Error::new(
                error.kind(),
                format!("cannot install skill {}: {error}", dir.display()),
            ));
        }
        writeln!(out, "Installed: {}", dir.display())?;
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
        // The rename error is the one returned; a failed rollback is added to it.
        if let Err(rollback) = fs::rename(&previous, dir) {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}; the previous copy at {} could not be restored: {rollback}",
                    previous.display()
                ),
            ));
        }
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
        let relative = path
            .split_once('/')
            .unwrap_or_else(|| panic!("bundled skill file `{path}` has no directory in its path"))
            .1;
        let destination = dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, content)?;
    }
    let license = match name {
        "jidoka-security-review" => "licenses/awesome-copilot-LICENSE",
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

    fn selected(
        agent: Option<&str>,
        env: &[(&str, &str)],
        terminal: bool,
        answers: &str,
    ) -> (io::Result<Vec<Agent>>, String) {
        let var = |name: &str| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        };
        let mut out = Vec::new();
        let result = select(agent, var, terminal, &mut answers.as_bytes(), &mut out);
        (result, String::from_utf8(out).unwrap())
    }

    fn names(selection: io::Result<Vec<Agent>>) -> Vec<&'static str> {
        selection.unwrap().into_iter().map(Agent::name).collect()
    }

    #[test]
    fn the_prompt_accepts_numbers_names_lists_and_all() {
        for (answer, expected) in [
            ("2\n", vec!["claude"]),
            ("copilot\n", vec!["copilot"]),
            ("1,claude\n", vec!["codex", "claude"]),
            ("3 1\n", vec!["copilot", "codex"]),
            ("all\n", vec!["codex", "claude", "copilot"]),
            ("2,claude\n", vec!["claude"]),
        ] {
            let (selection, _) = selected(None, &[], true, answer);
            assert_eq!(names(selection), expected, "{answer:?}");
        }
    }

    #[test]
    fn without_a_terminal_and_agents_the_error_names_the_flag_and_the_variable() {
        let (selection, shown) = selected(None, &[], false, "all\n");
        let message = selection.unwrap_err().to_string();
        assert!(message.contains("--agent") && message.contains("JIDOKA_AGENT"));
        assert!(shown.is_empty());
    }

    #[test]
    fn the_flag_wins_over_the_variable_and_neither_prompts() {
        let env = [("JIDOKA_AGENT", "codex")];
        let (selection, shown) = selected(Some("claude"), &env, true, "");
        assert_eq!(names(selection), ["claude"]);
        let (selection, shown_env) = selected(None, &env, false, "");
        assert_eq!(names(selection), ["codex"]);
        assert!(shown.is_empty() && shown_env.is_empty());
        let (selection, _) = selected(Some("nope"), &[], true, "");
        assert!(selection.is_err());
    }

    #[test]
    fn an_invalid_flag_or_variable_entry_names_the_token_and_the_valid_agents() {
        for value in ["codex,nope", "9", "0", "claude,,x1"] {
            let (selection, _) = selected(Some(value), &[], true, "");
            let message = selection.unwrap_err().to_string();
            let bad = value.rsplit(',').next().unwrap();
            assert!(
                message.contains(&format!("{bad:?}"))
                    && message.contains("codex, claude, or copilot"),
                "{value}: {message}"
            );
        }
        let (selection, _) = selected(None, &[("JIDOKA_AGENT", "1,zzz")], false, "");
        assert!(selection.unwrap_err().to_string().contains("\"zzz\""));
    }

    #[test]
    fn skills_root_honors_agent_config_directories() {
        let home = Path::new("/home/user");
        let unset = |_: &str| None;
        let set = |name: &str| Some(OsString::from(format!("/custom/{name}")));
        let empty = |_: &str| Some(OsString::new());
        let root = |agent, var: &dyn Fn(&str) -> Option<OsString>| skills_root(agent, home, var);
        assert_eq!(root(Agent::Claude, &unset), home.join(".claude/skills"));
        assert_eq!(root(Agent::Claude, &empty), home.join(".claude/skills"));
        assert_eq!(
            root(Agent::Claude, &set),
            Path::new("/custom/CLAUDE_CONFIG_DIR/skills")
        );
        assert_eq!(root(Agent::Codex, &unset), home.join(".codex/skills"));
        assert_eq!(
            root(Agent::Codex, &set),
            Path::new("/custom/CODEX_HOME/skills")
        );
        assert_eq!(root(Agent::Copilot, &unset), home.join(".copilot/skills"));
        assert_eq!(root(Agent::Copilot, &empty), home.join(".copilot/skills"));
        assert_eq!(
            root(Agent::Copilot, &set),
            Path::new("/custom/COPILOT_HOME/skills")
        );
    }
    /// Names that belong to this repository, not to the projects the skills are installed in.
    /// Concrete commands and branches go in AGENTS.md, README or .github/instructions instead.
    const PROJECT_SPECIFIC: [&str; 3] = ["verify.sh", "TDSchreur/jidoka", "for this project"];
    /// The one skill that files issues in this repository and so names it.
    const REPORTS_TO_THIS_REPOSITORY: &str = "jidoka-run-report";
    /// Workflow skills must also not assume this repository's branch name.
    const WORKFLOW_SKILLS: [&str; 3] = ["jidoka-full", "jidoka-balanced", "jidoka-fast"];

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
        let selector = bundled_text("jidoka/SKILL.md");
        assert_eq!(selector.lines().nth(1), Some("name: jidoka"));
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
        let text = ["run-reporting", "define-rules", "workspace"]
            .map(|chunk| bundled_text(&format!("jidoka-procedures/chunks/{chunk}.md")))
            .join("\n");
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
            "--unticketed",
            "--summary",
            "--pr",
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
            "jidoka run begin",
            "jidoka run inspect",
            "acceptance criteria",
        ] {
            assert!(saving.contains(needle), "saving the plan lacks {needle}");
        }
        for old in [
            "git directory",
            "git-dir",
            ".jidoka-plan.md",
            "beside the worktree",
        ] {
            assert!(
                !saving.contains(old),
                "saving the plan still mentions {old}"
            );
        }
        // Continuing: inspect first, verify the recorded state, and never discard work.
        let continuing = text
            .split_once("## Continuing a run")
            .expect("the shared procedures have no continuation protocol")
            .1;
        let continuing = continuing.split("\n## ").next().unwrap();
        for needle in [
            "jidoka run inspect",
            "jidoka run check",
            "recorded branch",
            "ancestor of `HEAD`",
            "unexpected changes",
            "Never reset, clean",
        ] {
            assert!(continuing.contains(needle), "continuation lacks {needle}");
        }
    }

    #[test]
    fn workflow_skills_bootstrap_the_stage_machine() {
        for skill in WORKFLOW_SKILLS {
            let text = bundled_text(&format!("{skill}/SKILL.md"));
            for instruction in [
                "../jidoka-procedures/SKILL.md",
                &format!("jidoka run begin \"<short task title>\" --agent <agent> --mode {skill}"),
                "jidoka run advance <id> --summary",
                "so no separate `jidoka run next` is needed",
            ] {
                assert!(text.contains(instruction), "{skill} lacks {instruction}");
            }
        }
        // The selector starts the run itself, so the agent need not read the workflow skill too.
        let selector = bundled_text("jidoka/SKILL.md");
        for instruction in ["jidoka run begin", "jidoka run advance <id> --summary"] {
            assert!(selector.contains(instruction), "jidoka lacks {instruction}");
        }
    }

    #[test]
    fn bundled_skills_carry_the_skill_check() {
        for (path, wording) in [
            (
                "jidoka-procedures/chunks/skill-change.md",
                &["Skill check:", "wording only"][..],
            ),
            (
                "jidoka-code-review/SKILL.md",
                &["Skill check:", "no commit carries a `Skill check:`"][..],
            ),
            ("jidoka-pr/SKILL.md", &["**Skill check:**"][..]),
            (
                "jidoka-procedures/chunks/model-routing.md",
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
    fn the_procedures_keep_the_pull_request_current_without_rewriting_history() {
        let section =
            bundled_text("jidoka-procedures/chunks/pull-request-and-ci.md").replace("\r\n", "\n");
        for needle in [
            "jidoka pr sync <run-id> --base <base>",
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
                // The run report skill files issues in this repository, so it must name it.
                if skill == REPORTS_TO_THIS_REPOSITORY && term == "TDSchreur/jidoka" {
                    continue;
                }
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
        for (path, content) in crate::run::next::STAGE_CHUNKS {
            let text = String::from_utf8_lossy(content).to_lowercase();
            for term in PROJECT_SPECIFIC.into_iter().chain(["develop"]) {
                if text
                    .split(|c: char| !c.is_alphanumeric() && c != '/' && c != '.')
                    .any(|word| word == term)
                    || (term != "develop" && text.contains(term))
                {
                    offenders.push(format!("stages/{path}: {term}"));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "bundled skills must not name this repository (see AGENTS.md): {offenders:?}"
        );
    }

    #[test]
    fn setup_links_exactly_the_bundled_issue_tracker_templates() {
        let setup = bundled_text("jidoka-setup/SKILL.md");
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
            .filter_map(|path| path.strip_prefix("jidoka-setup/"))
            .filter(|name| name.starts_with("issue-tracker-") && name.ends_with(".md"))
            .collect();
        bundled.sort_unstable();
        assert_eq!(linked, bundled);
        assert!(linked.contains(&"issue-tracker-azure-boards.md"));
    }

    #[test]
    fn setup_stops_on_missing_azure_boards_details_without_a_partial_tracker_file() {
        let setup = bundled_text("jidoka-setup/SKILL.md");
        let (_, azure) = setup
            .split_once("An Azure Boards tracker needs")
            .expect("setup has an Azure Boards part");
        assert!(azure.contains("is missing, stop and tell the user"));
        assert!(azure.contains("partial `docs/agents/issue-tracker.md`"));
    }

    #[test]
    fn skills_roots_resolves_every_listed_agent_once() {
        let home = Path::new("/home/user");
        let unset = |_: &str| None;
        assert_eq!(
            skills_roots(
                &parse_agent_choice("codex, claude,codex").unwrap(),
                home,
                unset
            ),
            [home.join(".codex/skills"), home.join(".claude/skills")]
        );
        assert_eq!(
            skills_roots(&[Agent::Copilot], home, unset),
            [home.join(".copilot/skills")]
        );
        let error = parse_agent_choice("codex, vscode").unwrap_err();
        assert!(error.to_string().contains("vscode"));
    }
    #[test]
    fn selector_replaces_full_and_all_entry_points_survive_reinstall_for_each_agent() {
        let home = tempfile::tempdir().unwrap();
        let roots = skills_roots(&Agent::ALL, home.path(), |_: &str| None);
        for root in roots {
            fs::create_dir_all(root.join("jidoka")).unwrap();
            fs::write(
                root.join("jidoka/SKILL.md"),
                bundled_text("jidoka-full/SKILL.md"),
            )
            .unwrap();
            for _ in 0..2 {
                install_to(&root).unwrap();
                for skill in ["jidoka", "jidoka-full", "jidoka-balanced", "jidoka-fast"] {
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
    #[expect(
        clippy::cognitive_complexity,
        reason = "one scenario checks install, replacement, stale removal and user skills"
    )]
    fn installs_bundled_skills_and_replaces_existing_ones() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join("jidoka")).unwrap();
        fs::write(root.join("jidoka/SKILL.md"), "old content").unwrap();
        fs::write(root.join("jidoka/stale.md"), "removed upstream").unwrap();
        fs::create_dir_all(root.join("my-skill")).unwrap();
        fs::write(root.join("my-skill/SKILL.md"), "user skill").unwrap();
        install_to(root).unwrap();
        assert!(
            fs::read_to_string(root.join("jidoka/SKILL.md"))
                .unwrap()
                .contains("name: jidoka")
        );
        assert!(!root.join("jidoka/stale.md").exists());
        let mut installed: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != "my-skill")
            .collect();
        installed.sort();
        assert_eq!(installed, bundled_skill_names());
        assert!(
            installed.iter().all(|name| name.starts_with("jidoka")),
            "{installed:?}"
        );
        assert_eq!(
            fs::read_to_string(root.join("my-skill/SKILL.md")).unwrap(),
            "user skill"
        );
        assert!(
            root.join("jidoka-security-review/references/language-patterns.md")
                .is_file()
        );
        assert!(root.join("jidoka-security-review/LICENSE").is_file());
        assert!(root.join("jidoka-setup/SKILL.md").is_file());
        assert!(root.join("jidoka-setup/issue-tracker-github.md").is_file());
        assert!(
            root.join("jidoka-setup/issue-tracker-azure-boards.md")
                .is_file()
        );
        assert!(root.join("jidoka-setup/LICENSE").is_file());
        assert!(root.join("jidoka-to-spec/SKILL.md").is_file());
        assert!(root.join("jidoka-full/SKILL.md").is_file());
        assert!(!root.join("jidoka-full/LICENSE").exists());
        assert!(root.join("jidoka-fast/SKILL.md").is_file());
        assert!(root.join("jidoka-implement-fast/SKILL.md").is_file());
        assert!(root.join("jidoka-tdd/mocking.md").is_file());
        assert!(!root.join("jidoka-fast/LICENSE").exists());
        assert!(root.join("jidoka-balanced/SKILL.md").is_file());
        assert!(root.join("jidoka-implement-balanced/SKILL.md").is_file());
        assert!(!root.join("jidoka-balanced/LICENSE").exists());
        assert!(root.join("jidoka-procedures/SKILL.md").is_file());
        assert!(
            root.join("jidoka-procedures/chunks/model-routing.md")
                .is_file()
        );
        assert!(
            root.join("jidoka-procedures/chunks/pull-request-and-ci.md")
                .is_file()
        );
        assert!(!root.join("jidoka-procedures/LICENSE").exists());
        assert!(!root.join("jidoka-implement-balanced/LICENSE").exists());
        assert!(!root.join("jidoka-implement-fast/LICENSE").exists());
        assert!(root.join("jidoka-triage/AGENT-BRIEF.md").is_file());
        assert!(root.join("jidoka-triage/LICENSE").is_file());
        assert!(root.join("jidoka-run-report/SKILL.md").is_file());
        assert!(!root.join("jidoka-run-report/LICENSE").exists());
        assert!(
            root.join("jidoka-awesome-copilot/references/target-layout.md")
                .is_file()
        );
        assert!(!root.join("jidoka-awesome-copilot/LICENSE").exists());
        assert!(!root.join("jidoka-setup-knowledge-repo/LICENSE").exists());
        assert!(!root.join("jidoka-cross-repo-change/LICENSE").exists());
        assert!(root.join("jidoka-work-through-plan/SKILL.md").is_file());
        assert!(!root.join("jidoka-work-through-plan/LICENSE").exists());
        assert!(root.join("jidoka-pr-watch/SKILL.md").is_file());
        assert!(
            root.join("jidoka-pr-watch/references/azure-devops.md")
                .is_file()
        );
        assert!(!root.join("jidoka-pr-watch/LICENSE").exists());
        install_to(root).unwrap();
        let leftovers: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "leftover staging dirs: {leftovers:?}");
    }
}
