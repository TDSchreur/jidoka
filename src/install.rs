use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));

/// Skills maintained in this project, installed without a third-party license.
const LOCAL_SKILLS: [&str; 7] = [
    "jidoka",
    "jidoka-full",
    "jidoka-procedures",
    "jidoka-fast",
    "jidoka-balanced",
    "jidoka-implement-fast",
    "jidoka-implement-balanced",
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

/// The agents and Claude config directories a `jidoka install` should use.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub agents: Vec<String>,
    pub config_dirs: Vec<String>,
}

/// The agents Jidoka installs skills for, in the order the install menu lists them.
pub(crate) const AGENTS: [&str; 3] = ["codex", "claude", "copilot"];

/// `codex, claude, or copilot`.
pub(crate) fn agent_list() -> String {
    let (last, rest) = AGENTS.split_last().expect("AGENTS is not empty");
    format!("{}, or {last}", rest.join(", "))
}

fn unknown_agent(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("unknown agent {name:?}; use {}", agent_list()),
    )
}

/// Turns an answer such as `1,claude`, `2 3` or `all` into agent names, without duplicates.
///
/// # Errors
///
/// Fails naming the first entry that is not a number, name or `all`, and on an empty answer.
pub(crate) fn parse_agent_choice(answer: &str) -> io::Result<Vec<String>> {
    let mut agents: Vec<String> = Vec::new();
    for entry in answer
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
    {
        let chosen: Vec<&str> = match entry {
            "all" => AGENTS.to_vec(),
            name if AGENTS.contains(&name) => vec![name],
            number => match number.parse::<usize>() {
                Ok(n) if (1..=AGENTS.len()).contains(&n) => vec![AGENTS[n - 1]],
                _ => return Err(unknown_agent(number)),
            },
        };
        for name in chosen {
            if !agents.iter().any(|known| known == name) {
                agents.push(name.to_string());
            }
        }
    }
    if agents.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no agent given; use {}", agent_list()),
        ));
    }
    Ok(agents)
}

/// Decides the agents and Claude config directories. `--agent` and `--config-dir` win over
/// `JIDOKA_AGENT` and `JIDOKA_CLAUDE_CONFIG_DIRS` (the platform path-list separator). With neither
/// agents source, asks on `output`/`input` when `terminal` is true and otherwise fails.
pub(crate) fn select(
    agent_flag: Option<&str>,
    dir_flags: &[String],
    var: impl Fn(&str) -> Option<OsString>,
    terminal: bool,
    input: &mut impl io::BufRead,
    output: &mut impl io::Write,
) -> io::Result<Selection> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    let env_agent = var("JIDOKA_AGENT")
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty());
    let agents = match agent_flag.map(str::to_string).or(env_agent) {
        Some(list) => parse_agent_choice(&list)?,
        None if terminal => ask_agents(input, output)?,
        None => {
            return Err(invalid(
                "No terminal to ask which agents to install for. Use --agent or set JIDOKA_AGENT, for example --agent codex,claude."
                    .to_string(),
            ));
        }
    };
    let mut config_dirs = dir_flags.to_vec();
    if config_dirs.is_empty()
        && agents.iter().any(|agent| agent == "claude")
        && let Some(list) = var("JIDOKA_CLAUDE_CONFIG_DIRS")
    {
        config_dirs = env::split_paths(&list)
            .map(|path| path.to_string_lossy().into_owned())
            .filter(|path| !path.is_empty())
            .collect();
    }
    Ok(Selection {
        agents,
        config_dirs,
    })
}

fn ask_agents(
    input: &mut impl io::BufRead,
    output: &mut impl io::Write,
) -> io::Result<Vec<String>> {
    loop {
        writeln!(output, "Install the Jidoka skills for which agents?")?;
        for (index, name) in AGENTS.iter().enumerate() {
            writeln!(output, "  {}) {name}", index + 1)?;
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

/// `jidoka install [--agent <list>] [--config-dir <path>]...`: installs the skills and the default
/// routing policy, then writes the install record.
pub(crate) fn run(options: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    use io::IsTerminal;
    let (agent, dirs) = parse_options(options)?;
    let stdin = io::stdin();
    let selection = select(
        agent,
        &dirs,
        |name| env::var_os(name),
        stdin.is_terminal(),
        &mut stdin.lock(),
        &mut io::stdout(),
    )?;
    install(&selection)?;
    Ok(())
}

/// Installs the bundled skills for the selected agents, the default policy and the install record.
/// The config dirs need `claude` among the agents.
pub(crate) fn install(selection: &Selection) -> io::Result<()> {
    let var = |name: &str| env::var_os(name);
    install_skills(selection, &home_dir()?, &var, &mut io::stdout())?;
    let path = crate::install_record::path_in(var).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot find Jidoka config directory for the install record",
        )
    })?;
    crate::install_record::write(
        &path,
        &crate::install_record::InstallRecord::current(
            selection.agents.clone(),
            selection.config_dirs.clone(),
        ),
    )
}

/// The skills and the default policy of an install, without the install record; policy messages go to `out`.
pub(crate) fn install_skills(
    selection: &Selection,
    home: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
    out: &mut impl io::Write,
) -> io::Result<()> {
    install_skill_files(selection, home, var, out)?;
    ensure_default_policy(var, out)
}

/// Only the skills of an install: no policy, no record.
pub(crate) fn install_skill_files(
    selection: &Selection,
    home: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
    out: &mut impl io::Write,
) -> io::Result<()> {
    let roots = skills_roots(&selection.agents, &selection.config_dirs, home, var)?;
    for root in roots {
        install_to_reporting(&root, out)?;
    }
    Ok(())
}

/// Create the initial routing policy and report what happened on `out`.
fn ensure_default_policy(
    var: impl Fn(&str) -> Option<OsString>,
    out: &mut impl io::Write,
) -> io::Result<()> {
    let (path, created) = create_default_policy(var)?;
    if created {
        let shown = path.display().to_string();
        writeln!(out, "{}", crate::routing::created_message(&shown, None))?;
    }
    Ok(())
}

/// Create the initial routing policy unless a file is already there.
pub(crate) fn create_default_policy(
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<(PathBuf, bool)> {
    let path = crate::routing::policy_path_in(var).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot find Jidoka config directory for model routing",
        )
    })?;
    let agents = [
        crate::routing::RoutingAgent::Claude,
        crate::routing::RoutingAgent::Codex,
        crate::routing::RoutingAgent::Copilot,
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
    Ok((path, created))
}

/// Resolves the skills folder of every listed agent before anything is installed.
pub(crate) fn skills_roots(
    agents: &[impl AsRef<str>],
    config_dirs: &[String],
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<Vec<PathBuf>> {
    let names: Vec<&str> = agents.iter().map(AsRef::as_ref).collect();
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
        _ => Err(unknown_agent(agent)),
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
        dirs: &[String],
        env: &[(&str, &str)],
        terminal: bool,
        answers: &str,
    ) -> (io::Result<Selection>, String) {
        let var = |name: &str| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        };
        let mut out = Vec::new();
        let result = select(
            agent,
            dirs,
            var,
            terminal,
            &mut answers.as_bytes(),
            &mut out,
        );
        (result, String::from_utf8(out).unwrap())
    }

    fn names(selection: io::Result<Selection>) -> Vec<String> {
        selection.unwrap().agents
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
            let (selection, _) = selected(None, &[], &[], true, answer);
            assert_eq!(names(selection), expected, "{answer:?}");
        }
    }

    #[test]
    fn without_a_terminal_and_agents_the_error_names_the_flag_and_the_variable() {
        let (selection, shown) = selected(None, &[], &[], false, "all\n");
        let message = selection.unwrap_err().to_string();
        assert!(message.contains("--agent") && message.contains("JIDOKA_AGENT"));
        assert!(shown.is_empty());
    }

    #[test]
    fn the_flag_wins_over_the_variable_and_neither_prompts() {
        let env = [("JIDOKA_AGENT", "codex")];
        let (selection, shown) = selected(Some("claude"), &[], &env, true, "");
        assert_eq!(names(selection), ["claude"]);
        let (selection, shown_env) = selected(None, &[], &env, false, "");
        assert_eq!(names(selection), ["codex"]);
        assert!(shown.is_empty() && shown_env.is_empty());
        let (selection, _) = selected(Some("nope"), &[], &[], true, "");
        assert!(selection.is_err());
    }

    #[test]
    fn an_invalid_flag_or_variable_entry_names_the_token_and_the_valid_agents() {
        for value in ["codex,nope", "9", "0", "claude,,x1"] {
            let (selection, _) = selected(Some(value), &[], &[], true, "");
            let message = selection.unwrap_err().to_string();
            let bad = value.rsplit(',').next().unwrap();
            assert!(
                message.contains(&format!("{bad:?}"))
                    && message.contains("codex, claude, or copilot"),
                "{value}: {message}"
            );
        }
        let (selection, _) = selected(None, &[], &[("JIDOKA_AGENT", "1,zzz")], false, "");
        assert!(selection.unwrap_err().to_string().contains("\"zzz\""));
    }

    #[test]
    fn config_dirs_come_from_flags_else_the_path_list_variable() {
        let list = env::join_paths([test_abs("a"), test_abs("b c")]).unwrap();
        let list = list.to_str().unwrap();
        let env = [("JIDOKA_CLAUDE_CONFIG_DIRS", list)];
        let (selection, _) = selected(Some("claude"), &[], &env, false, "");
        assert_eq!(
            selection.unwrap().config_dirs,
            [test_abs("a"), test_abs("b c")].map(|p| p.to_string_lossy().into_owned())
        );
        let flag = vec![test_abs("flag").to_string_lossy().into_owned()];
        let (selection, _) = selected(Some("claude"), &flag, &env, false, "");
        assert_eq!(selection.unwrap().config_dirs, flag);
        let (selection, _) = selected(Some("codex"), &[], &env, false, "");
        assert!(selection.unwrap().config_dirs.is_empty());
    }

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
    const PROJECT_SPECIFIC: [&str; 3] = ["verify.sh", "TDSchreur/jidoka", "for this project"];
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
                "jidoka run next",
                "jidoka run advance",
            ] {
                assert!(text.contains(instruction), "{skill} lacks {instruction}");
            }
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
        let section = bundled_text("jidoka-procedures/references/pull-request-and-ci.md")
            .replace("\r\n", "\n");
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
        for (path, content) in crate::run_next::STAGE_CHUNKS {
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
                &[],
                home,
                unset
            )
            .unwrap(),
            [home.join(".codex/skills"), home.join(".claude/skills")]
        );
        assert_eq!(
            skills_roots(&["copilot"], &[], home, unset).unwrap(),
            [home.join(".copilot/skills")]
        );
        let error = skills_roots(&["codex", "vscode"], &[], home, unset).unwrap_err();
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
    fn selector_replaces_full_and_all_entry_points_survive_reinstall_for_each_agent() {
        let home = tempfile::tempdir().unwrap();
        let roots = skills_roots(
            &["codex", "claude", "copilot"],
            &[],
            home.path(),
            |_: &str| None,
        )
        .unwrap();
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
            root.join("jidoka-procedures/references/pull-request-and-ci.md")
                .is_file()
        );
        assert!(!root.join("jidoka-procedures/LICENSE").exists());
        assert!(!root.join("jidoka-implement-balanced/LICENSE").exists());
        assert!(!root.join("jidoka-implement-fast/LICENSE").exists());
        assert!(root.join("jidoka-triage/AGENT-BRIEF.md").is_file());
        assert!(root.join("jidoka-triage/LICENSE").is_file());
        install_to(root).unwrap();
        let leftovers: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "leftover staging dirs: {leftovers:?}");
    }
}
