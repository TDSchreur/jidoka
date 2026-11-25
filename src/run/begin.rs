//! `jidoka run begin`: creates a run and its workspace (a worktree on a new branch from the base) and records it.

use crate::{
    agent::Agent,
    git::{self, first_line, last_line, valid_branch_name},
    gitignore::ensure_ignored,
    routing::ModelRouting,
    run::inspect::shown,
};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

/// The Jidoka server, as far as this command needs it. Tests answer from a script.
pub(crate) trait RunApi {
    /// Creates a run and returns its id.
    fn create(
        &self,
        title: &str,
        project: &str,
        repository: Option<String>,
        session: Option<&AgentSession>,
        routing: Option<ModelRouting>,
    ) -> Result<String, String>;
    /// Reads a run that exists.
    fn get(&self, id: &str) -> Result<RunSummary, String>;
    fn workspace_ready(&self, id: &str, options: &[&str]) -> Result<(), String>;
    fn workspace_blocked(&self, id: &str, reason: &str) -> Result<(), String>;
    /// The id and title of every run, whatever its status, that records this agent session.
    fn runs_with_session(&self, session: &str) -> Result<Vec<(String, String)>, String>;
}

/// What `begin --run` needs to know about a run.
#[derive(Clone, Debug)]
pub(crate) struct RunSummary {
    pub(crate) title: String,
    pub(crate) finished: bool,
    /// The worktree the run records, if it has one already.
    pub(crate) worktree: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Options {
    /// Resume this run instead of creating one.
    pub(crate) run: Option<String>,
    pub(crate) title: String,
    pub(crate) agent: Option<Agent>,
    pub(crate) mode: Option<String>,
    pub(crate) base: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) json: bool,
}

/// The options of `jidoka run begin --run <id>`: the same, without a title.
pub(crate) fn parse_resume(run: &str, options: &[&str]) -> Result<Options, String> {
    crate::client::check_run_id(run)?;
    let mut parsed = parse_common("", options, false)?;
    parsed.run = Some(run.to_string());
    Ok(parsed)
}

/// `--agent`, `--mode`, `--base`, `--branch` and `--json` after the title, each at most once.
pub(crate) fn parse_options(title: &str, options: &[&str]) -> Result<Options, String> {
    parse_common(title, options, true)
}

fn parse_common(title: &str, options: &[&str], needs_title: bool) -> Result<Options, String> {
    if needs_title && title.trim().is_empty() {
        return Err("A run needs a title.".into());
    }
    let mut parsed = Options {
        title: title.to_string(),
        ..Options::default()
    };
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .copied()
                .ok_or_else(|| format!("{flag} needs a value."))
        };
        let repeated = |flag: &str| format!("{flag} was given twice.");
        match *arg {
            "--json" if !parsed.json => parsed.json = true,
            "--agent" if parsed.agent.is_none() => {
                let name = value("--agent")?;
                let agent = Agent::parse(name)
                    .ok_or_else(|| format!("Unknown agent {name:?}; use {}.", Agent::list()))?;
                parsed.agent = Some(agent);
            }
            "--mode" if parsed.mode.is_none() => {
                let mode = value("--mode")?;
                serde_json::from_value::<crate::run::Mode>(serde_json::Value::from(mode)).map_err(
                    |_| {
                        format!(
                            "Invalid value for --mode: {mode}; use full, balanced or fast (also accepted: jidoka-full, jidoka-balanced, jidoka-fast)"
                        )
                    },
                )?;
                parsed.mode = Some(mode.to_string());
            }
            "--base" if parsed.base.is_none() => {
                let base = value("--base")?;
                if !valid_branch_name(base) {
                    return Err(format!("{base:?} is not a usable branch name for --base."));
                }
                parsed.base = Some(base.to_string());
            }
            "--branch" if parsed.branch.is_none() => {
                let branch = value("--branch")?;
                if !valid_branch_name(branch) {
                    return Err(format!(
                        "{branch:?} is not a usable branch name for --branch."
                    ));
                }
                parsed.branch = Some(branch.to_string());
            }
            "--json" | "--agent" | "--mode" | "--base" | "--branch" => {
                return Err(repeated(arg));
            }
            other => return Err(format!("Unexpected option {other:?}.")),
        }
    }
    Ok(parsed)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Begun {
    pub(crate) run: String,
    pub(crate) worktree: String,
    pub(crate) branch: String,
    pub(crate) base: String,
    pub(crate) base_commit: String,
    /// The entries `begin` added to the worktree's `.gitignore`.
    pub(crate) gitignore_added: Vec<String>,
    /// Why `.gitignore` could not be updated; the run itself is fine.
    pub(crate) gitignore_warning: Option<String>,
    /// Set when this agent session is already recorded on another run; the run itself is fine.
    pub(crate) session_warning: Option<String>,
}

/// What `begin --json` prints: the workspace and the path the run's plan is saved at.
#[derive(Serialize)]
pub(crate) struct Output<'a> {
    #[serde(flatten)]
    pub(crate) begun: &'a Begun,
    pub(crate) plan: String,
    /// The Define stage's `run next` data; none when a session warning is printed or it could not be read.
    pub(crate) next: Option<&'a crate::run::next::Next>,
}

/// Why no workspace was made.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BeginError {
    /// Nothing was created.
    NoRun(String),
    /// The run exists and is marked blocked with this reason.
    Blocked {
        run: String,
        reason: String,
        retry: String,
        /// Whether the server accepted the blocked status; if not, the run still shows as running.
        marked: bool,
    },
}

/// Reasons are kept short: the dashboard shows one on the run's card.
const MAX_REASON_CHARS: usize = 300;
/// The highest numeric suffix tried when a derived run name is taken: a hundred clashes means something is wrong.
const MAX_NAME_SUFFIX: u32 = 99;
/// A derived branch name is cut to this length, so it stays short to type and read.
const MAX_NAME_CHARS: usize = 40;
/// What every run's worktree keeps out of git: Jidoka's own folder and the local tracker.
const RUN_IGNORED: &[&str] = &[".jidoka/", ".scratch/"];

/// Adds the run folders to the worktree's `.gitignore`: the entries added and, when the file could not be
/// updated, a warning (the run itself is fine).
fn ignore_run_folders(worktree: &Path) -> (Vec<String>, Option<String>) {
    match ensure_ignored(worktree, RUN_IGNORED) {
        Ok(added) => (added, None),
        Err(error) => (
            Vec::new(),
            Some(format!(
                "Could not update the worktree's .gitignore: {error}"
            )),
        ),
    }
}

/// A kebab-case name from a title: ASCII letters and digits, at most `MAX_NAME_CHARS` characters.
fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(MAX_NAME_CHARS);
    out.trim_end_matches('-').to_string()
}

/// The command that resumes this run with the same choices.
fn retry_command(run: &str, options: &Options, named: &Named) -> String {
    let branch = match named {
        Named::Unknown => options.branch.clone(),
        Named::Chosen(name) => Some(name.clone()),
        Named::Taken => None,
    };
    let mut command = format!("jidoka run begin --run {run}");
    let agent = options.agent.map(|agent| agent.name().to_string());
    for (flag, value) in [
        ("--agent", &agent),
        ("--mode", &options.mode),
        ("--base", &options.base),
        ("--branch", &branch),
    ] {
        if let Some(value) = value {
            command.push_str(&format!(" {flag} {value}"));
        }
    }
    command
}

/// The agent session a command runs in, and which agent it belongs to when that is known.
#[derive(Clone, Debug)]
pub(crate) struct AgentSession {
    pub(crate) id: String,
    pub(crate) agent: Option<Agent>,
    /// The agent's home folder in the environment the command runs in, when it is an absolute path.
    pub(crate) home: Option<String>,
}

impl AgentSession {
    /// The `--session` option for this session, and `--agent-home` when its agent and home are known.
    pub(crate) fn update_options(&self) -> Vec<String> {
        let mut options = vec!["--session".to_owned(), self.id.clone()];
        if let (Some(agent), Some(home)) = (self.agent, &self.home) {
            options.extend([
                "--agent-home".to_owned(),
                format!("{}={home}", agent.name()),
            ]);
        }
        options
    }
}

/// The home folder of each agent the sessions belong to, keyed by agent, for the run record.
pub(crate) fn agent_homes(sessions: &[AgentSession]) -> BTreeMap<String, String> {
    sessions
        .iter()
        .filter_map(|session| Some((session.agent?.name().to_owned(), session.home.clone()?)))
        .collect()
}

/// What to do in `agent` to get a fresh conversation, for the session warning.
fn fresh_session_advice(agent: Option<Agent>) -> &'static str {
    match agent {
        Some(Agent::Claude) => {
            "Start each run in a fresh agent session: use /clear in Claude Code."
        }
        Some(Agent::Codex) => "Start each run in a fresh agent session: use /new in Codex.",
        Some(Agent::Copilot) => {
            "Start each run in a fresh agent session: use /clear in Copilot CLI."
        }
        None => "Start each run in a fresh agent session.",
    }
}

/// A warning when `session` is already recorded on a run other than `resumed`. Any failure to list runs
/// gives no warning.
fn session_warning(
    api: &dyn RunApi,
    session: Option<&AgentSession>,
    resumed: Option<&str>,
) -> Option<String> {
    let session = session?;
    let others: Vec<String> = api
        .runs_with_session(&session.id)
        .ok()?
        .into_iter()
        .filter(|(id, _)| Some(id.as_str()) != resumed)
        .map(|(id, title)| format!("{id} ({title})"))
        .collect();
    if others.is_empty() {
        return None;
    }
    Some(format!(
        "This agent session is already recorded on run {}. {}",
        others.join(", "),
        fresh_session_advice(session.agent)
    ))
}

/// Creates the run and its workspace. Never deletes, resets, checks out over or cleans anything.
pub(crate) fn begin(
    dir: &Path,
    options: &Options,
    api: &dyn RunApi,
    session: Option<AgentSession>,
    routing: Option<ModelRouting>,
) -> Result<Begun, BeginError> {
    let Some(repository) = crate::repository::canonical_repository(dir) else {
        return Err(BeginError::NoRun(
            "This folder is not in a git repository (or git is not installed). Run the command in the project's checkout."
                .into(),
        ));
    };
    if Path::new(&repository)
        .components()
        .any(|part| part.as_os_str() == ".git")
    {
        return Err(BeginError::NoRun(
            "This repository's git folder is inside another .git folder (a submodule?), so worktrees cannot go beside it. Run the command in a normal checkout, or create the worktree by hand."
                .into(),
        ));
    }
    let session_warning = session_warning(api, session.as_ref(), options.run.as_deref());
    // A warned session is told to stop, so the run must not claim its usage; the fresh session records itself.
    let session = session.filter(|_| session_warning.is_none());
    let (run, title) = match &options.run {
        Some(id) => {
            let known = api.get(id).map_err(BeginError::NoRun)?;
            if known.finished {
                return Err(BeginError::NoRun(format!("Run {id} is finished.")));
            }
            if let Some(worktree) = known.worktree {
                return Err(BeginError::NoRun(format!(
                    "Run {id} already has a workspace at {worktree}."
                )));
            }
            (id.clone(), known.title)
        }
        None => {
            let run = api
                .create(
                    &options.title,
                    &dir.display().to_string(),
                    Some(repository.clone()),
                    session.as_ref(),
                    routing,
                )
                .map_err(BeginError::NoRun)?;
            (run, options.title.clone())
        }
    };
    match workspace(
        Path::new(&repository),
        options,
        &title,
        &run,
        options.run.is_some(),
    ) {
        Ok(workspace) => {
            let mut args: Vec<&str> = Vec::new();
            if let Some(mode) = &options.mode {
                args.extend(["--mode", mode]);
            }
            args.extend([
                "--worktree",
                &workspace.worktree,
                "--branch",
                &workspace.branch,
                "--base-commit",
                &workspace.base_commit,
                "--activity",
                "Workspace ready",
            ]);
            let session_options = session
                .as_ref()
                .map(AgentSession::update_options)
                .unwrap_or_default();
            args.extend(session_options.iter().map(String::as_str));
            match api.workspace_ready(&run, &args) {
                Ok(()) => {
                    let (gitignore_added, gitignore_warning) =
                        ignore_run_folders(Path::new(&workspace.worktree));
                    Ok(Begun {
                        run,
                        worktree: workspace.worktree,
                        branch: workspace.branch,
                        base: workspace.base,
                        base_commit: workspace.base_commit,
                        gitignore_added,
                        gitignore_warning,
                        session_warning,
                    })
                }
                Err(error) => Err(blocked(
                    api,
                    &run,
                    options,
                    &Named::Chosen(workspace.branch.clone()),
                    format!(
                        "The workspace is at {}, but the run could not be updated: {error}",
                        workspace.worktree
                    ),
                )),
            }
        }
        Err(failure) => Err(blocked(api, &run, options, &failure.named, failure.reason)),
    }
}

/// Marks the run blocked with `reason` and returns the error that says so.
fn blocked(
    api: &dyn RunApi,
    run: &str,
    options: &Options,
    named: &Named,
    reason: String,
) -> BeginError {
    let reason: String = reason.chars().take(MAX_REASON_CHARS).collect();
    let marked = api.workspace_blocked(run, &reason).is_ok();
    BeginError::Blocked {
        run: run.to_string(),
        reason,
        retry: retry_command(run, options, named),
        marked,
    }
}

/// What was made.
struct Workspace {
    worktree: String,
    branch: String,
    base: String,
    base_commit: String,
}

/// What a failed attempt knows about the branch name, for the retry command.
enum Named {
    /// No name was settled on yet; nothing was made, so the retry repeats what was asked for.
    Unknown,
    /// This name is the attempt's own: the retry takes over what it left.
    Chosen(String),
    /// The name asked for belongs to something else. The retry must not repeat it, or it would take that over.
    Taken,
}

/// Why the workspace was not made.
struct Failure {
    reason: String,
    named: Named,
}

impl From<String> for Failure {
    fn from(reason: String) -> Self {
        Failure {
            reason,
            named: Named::Unknown,
        }
    }
}

fn branch_exists(main: &Path, branch: &str) -> bool {
    git::run(
        main,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
}

/// The branch checked out in the worktree git lists at `path`: `None` if git lists none there, and
/// `Some(None)` if that one is on no branch.
fn worktree_branch(main: &Path, path: &Path) -> Option<Option<String>> {
    let wanted = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    git::worktrees(&git::RealGit, main)
        .ok()?
        .into_iter()
        .find(|worktree| {
            let location = Path::new(&worktree.path);
            location
                .canonicalize()
                .unwrap_or_else(|_| location.to_path_buf())
                == wanted
                || location == path
        })
        .map(|worktree| worktree.branch)
}

/// Whether `branch` has no commits of its own: everything on it is already on `origin/<base>`.
fn is_unused(main: &Path, branch: &str, base: &str) -> bool {
    git::run(
        main,
        &[
            "merge-base",
            "--is-ancestor",
            &format!("refs/heads/{branch}"),
            &format!("refs/remotes/origin/{base}"),
        ],
    )
    .is_ok()
}

/// Adds the worktree. `create` makes the branch too, from `origin/<base>`.
fn add_worktree(
    main: &Path,
    path: &Path,
    branch: &str,
    base: &str,
    create: bool,
) -> Result<(), String> {
    let path_text = path.display().to_string();
    let start = format!("origin/{base}");
    let args: Vec<&str> = if create {
        vec!["worktree", "add", "-b", branch, &path_text, &start]
    } else {
        vec!["worktree", "add", &path_text, branch]
    };
    git::run(main, &args).map(|_| ()).map_err(|error| {
        if error.to_lowercase().contains("permission denied") || error.to_lowercase().contains("read-only") {
            format!(
                "Git has no permission to write there ({}). Run where the repository's git folder and {} are writable, or ask the user to create the worktree.",
                last_line(&error),
                path.parent().unwrap_or(path).display()
            )
        } else {
            format!("Creating the worktree at {path_text} failed: {}", last_line(&error))
        }
    })
}

/// Fetches the base and adds the worktree, or on a resume finishes what an earlier attempt began. The
/// failure is a reason that says what to do. Nothing is ever deleted, reset, checked out over or cleaned.
#[expect(
    clippy::too_many_lines,
    reason = "picks a branch name, resumes or creates the worktree and reports each failure"
)]
fn workspace(
    main: &Path,
    options: &Options,
    title: &str,
    run: &str,
    resume: bool,
) -> Result<Workspace, Failure> {
    let parent = main.parent().unwrap_or(main);
    let repo_name = main
        .file_name()
        .map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
    let folder = parent.join(format!("{repo_name}.worktrees"));
    let path_of = |branch: &str| folder.join(branch.replace('/', "-"));
    let taken = |branch: &str| branch_exists(main, branch) || path_of(branch).exists();

    // A name that was asked for and is taken is found before anything else can fail, so that the retry
    // command printed for any later failure cannot repeat it and take over what holds it.
    if let (false, Some(wanted)) = (resume, &options.branch) {
        let reason = if branch_exists(main, wanted) {
            Some(format!(
                "The branch {wanted} already exists. Pick another name with --branch."
            ))
        } else if path_of(wanted).exists() {
            Some(format!(
                "{} already exists. Pick another name with --branch.",
                path_of(wanted).display()
            ))
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(Failure {
                reason,
                named: Named::Taken,
            });
        }
    }
    git::run(main, &["remote", "get-url", "origin"]).map_err(|_| {
        "This repository has no remote named origin. Add one with `git remote add origin <url>` and retry."
            .to_string()
    })?;
    let base = match &options.base {
        Some(base) => base.clone(),
        None => git::run(main, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .ok()
            .and_then(|head| head.strip_prefix("origin/").map(str::to_string))
            .ok_or_else(|| {
                "origin/HEAD is not set, so the base branch is not known. Pass it with --base, or run `git remote set-head origin -a`."
                    .to_string()
            })?,
    };
    // An explicit refspec also works in a clone that was made for one branch only.
    let refspec = format!("+refs/heads/{base}:refs/remotes/origin/{base}");
    git::run(main, &["fetch", "origin", &refspec]).map_err(|error| {
        if error.contains("couldn't find remote ref") {
            format!("The base branch {base} does not exist on origin. Pass an existing branch with --base.")
        } else {
            format!("Fetching {base} from origin failed: {}", first_line(&error))
        }
    })?;
    let base_commit = git::run(
        main,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/origin/{base}^{{commit}}"),
        ],
    )
    .map_err(|error| {
        format!(
            "origin/{base} could not be read after fetching: {}",
            first_line(&error)
        )
    })?;

    let derived = {
        let name = slug(title);
        if name.is_empty() {
            format!("run-{run}")
        } else {
            name
        }
    };

    // The branch: the one asked for, or the first derived name, with -2, -3 and so on, that neither a
    // branch nor a folder uses. A resume takes over what exists only when it is told the branch: a retry
    // command names it as soon as one was chosen, and before that nothing was made.
    let adopt = resume && options.branch.is_some();
    let branch = match &options.branch {
        Some(branch) => branch.clone(),
        None => {
            let mut candidate = derived.clone();
            let mut number = 1;
            while taken(&candidate) {
                number += 1;
                if number > MAX_NAME_SUFFIX {
                    return Err(format!(
                        "Every name from {derived} to {derived}-{MAX_NAME_SUFFIX} is taken."
                    )
                    .into());
                }
                candidate = format!("{derived}-{number}");
            }
            candidate
        }
    };
    let fail = |reason: String| Failure {
        reason,
        named: Named::Chosen(branch.clone()),
    };
    let taken_name = |reason: String| Failure {
        reason,
        named: Named::Taken,
    };
    let path = path_of(&branch);
    let path_text = path.display().to_string();
    let exists = branch_exists(main, &branch);
    let listed = worktree_branch(main, &path);

    let made = |base_commit: String| Workspace {
        worktree: path_text.clone(),
        branch: branch.clone(),
        base: base.clone(),
        base_commit,
    };
    let tip = |name: &str| {
        git::run(
            main,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{name}^{{commit}}"),
            ],
        )
        .map_err(|error| {
            fail(format!(
                "The branch {name} could not be read: {}",
                first_line(&error)
            ))
        })
    };
    let own_commits = || {
        fail(format!(
            "The branch {branch} has commits that are not on origin/{base}, so it is not an empty branch from a failed attempt. Keep working on it by hand, or pick another name with --branch."
        ))
    };

    if !adopt {
        if exists {
            return Err(taken_name(format!(
                "The branch {branch} already exists. Pick another name with --branch."
            )));
        }
        if path.exists() {
            return Err(taken_name(format!(
                "{path_text} already exists. Pick another name with --branch."
            )));
        }
        add_worktree(main, &path, &branch, &base, true).map_err(fail)?;
        return Ok(made(base_commit));
    }

    // A resume: look at what an earlier attempt left.
    match (exists, path.exists(), listed) {
        (false, false, None) => {
            add_worktree(main, &path, &branch, &base, true).map_err(fail)?;
            Ok(made(base_commit))
        }
        (true, false, None) => {
            if !is_unused(main, &branch, &base) {
                return Err(own_commits());
            }
            let start = tip(&branch)?;
            add_worktree(main, &path, &branch, &base, false).map_err(fail)?;
            Ok(made(start))
        }
        (true, true, Some(Some(on))) if on == branch => {
            if !is_unused(main, &branch, &base) {
                return Err(own_commits());
            }
            let status = git::run(&path, &["status", "--porcelain"]).map_err(|error| {
                fail(format!(
                    "{path_text} could not be read: {}",
                    first_line(&error)
                ))
            })?;
            if !status.is_empty() {
                return Err(fail(format!(
                    "The worktree {path_text} has uncommitted changes, so it is not an unused workspace. Keep working in it by hand, or pick another name with --branch."
                )));
            }
            Ok(made(tip(&branch)?))
        }
        (_, false, Some(_)) => Err(fail(format!(
            "git lists a worktree at {path_text} that is not on disk any more. Run `git worktree prune` yourself, then retry."
        ))),
        (_, true, None) => Err(fail(format!(
            "{path_text} already exists and is not a worktree. Move it away or pick another name with --branch."
        ))),
        (_, _, Some(Some(on))) => Err(fail(format!(
            "git lists a worktree at {path_text} on the branch {on}, not on {branch}. Pick another name with --branch."
        ))),
        (_, _, Some(None)) => Err(fail(format!(
            "git lists a worktree at {path_text} that is on no branch. Pick another name with --branch."
        ))),
    }
}

/// The run and where its workspace is, as text; the run first.
pub(crate) fn render(begun: &Begun, plan: &str) -> String {
    let mut text = format!(
        "Run: {}\nWorktree: {}\nBranch: {}\nBase: {}\nBase commit: {}\n",
        shown(&begun.run),
        shown(&begun.worktree),
        shown(&begun.branch),
        shown(&begun.base),
        shown(&begun.base_commit)
    );
    text.push_str(&format!("Plan: {}\n", shown(plan)));
    if !begun.gitignore_added.is_empty() {
        text.push_str(&format!(
            "Added to .gitignore (commit it with the first commit): {}\n",
            begun.gitignore_added.join(", ")
        ));
    }
    if let Some(warning) = &begun.gitignore_warning {
        text.push_str(&format!("Warning: {}\n", shown(warning)));
    }
    if let Some(warning) = &begun.session_warning {
        text.push_str(&format!("Warning: {}\n", shown(warning)));
    }
    text
}

/// `jidoka run start <title> [--agent <agent>]`: creates a run without a workspace and prints its id.
pub(crate) struct StartCommand;

impl crate::cli::Command for StartCommand {
    /// The title, and the agent the policy is read for.
    type Options = (String, Option<Agent>);
    /// The new run's id.
    type Outcome = String;

    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn std::error::Error>> {
        let (title, options) = crate::cli::split_first(words);
        let host = crate::routing::parse_start_options(options).map_err(crate::cli::usage_error)?;
        Ok((title.to_owned(), host))
    }

    fn run((title, host): &Self::Options) -> Result<String, Box<dyn std::error::Error>> {
        let dir = std::env::current_dir()?;
        let repository = crate::repository::canonical_repository(&dir);
        let resolution = resolve_routing(*host)?;
        crate::client::start_run(
            &crate::client::address()?,
            &crate::client::token_file()?,
            title,
            &dir.display().to_string(),
            repository,
            current_agent_sessions().first(),
            resolution.selection,
        )
    }

    fn render(
        _: &Self::Options,
        id: String,
    ) -> Result<crate::cli::Output, Box<dyn std::error::Error>> {
        Ok(crate::cli::Output::text(format!("{id}\n")))
    }
}

/// Reads the model-routing policy once, before a run exists: a mistake in it, or a policy for several
/// agents without `--agent`, stops the start. Prints the notice, if any.
fn resolve_routing(host: Option<Agent>) -> Result<crate::routing::Resolution, String> {
    let resolution = crate::routing::policy_path_in(|name| std::env::var_os(name)).map_or_else(
        || Ok(crate::routing::Resolution::default()),
        |path| crate::routing::load(&path, host).map_err(|error| shown(&error)),
    )?;
    if let Some(notice) = &resolution.notice {
        eprintln!("{notice}");
    }
    Ok(resolution)
}

/// `jidoka run begin <title>`, or `--run <id>` to resume: creates a run and its workspace.
pub(crate) struct BeginCommand;

/// What `run begin` did: the run it began, with its plan path and Define instructions, or the run it left blocked.
#[expect(
    clippy::large_enum_variant,
    reason = "made once per command and moved straight into render"
)]
pub(crate) enum BeginOutcome {
    Begun {
        begun: Begun,
        plan: String,
        next: Option<crate::run::next::Next>,
    },
    /// The run is blocked; the message says why and how to retry.
    Blocked(String),
}

impl crate::cli::Command for BeginCommand {
    type Options = Options;
    type Outcome = BeginOutcome;

    fn parse(words: &[&str]) -> Result<Options, Box<dyn std::error::Error>> {
        let parsed = match words {
            ["--run", id, options @ ..] => parse_resume(id, options),
            [title, options @ ..] => parse_options(title, options),
            [] => Err("Invalid arguments.".to_owned()),
        };
        Ok(parsed.map_err(crate::cli::usage_error)?)
    }

    fn run(parsed: &Options) -> Result<BeginOutcome, Box<dyn std::error::Error>> {
        let dir = std::env::current_dir()?;
        // A resumed run already has its model routing; the policy is read only when a run is created.
        let resolution = if parsed.run.is_some() {
            crate::routing::Resolution::default()
        } else {
            resolve_routing(parsed.agent)?
        };
        match begin(
            &dir,
            parsed,
            &crate::client::ServerApi,
            current_agent_sessions().into_iter().next(),
            resolution.selection,
        ) {
            Ok(begun) => {
                let plan = crate::client::plan_path(&begun.run)?
                    .to_string_lossy()
                    .into_owned();
                // The Define instructions save the agent a `run next`; a session warning goes first.
                let next = if begun.session_warning.is_some() {
                    None
                } else {
                    match crate::run::next::next_for(&begun.run) {
                        Ok(next) => Some(next),
                        Err(error) => {
                            eprintln!(
                                "Warning: could not read the Define instructions ({}); run `jidoka run next {}`.",
                                shown(&error.to_string()),
                                begun.run
                            );
                            None
                        }
                    }
                };
                Ok(BeginOutcome::Begun { begun, plan, next })
            }
            Err(BeginError::NoRun(text)) => Err(shown(&text).into()),
            Err(BeginError::Blocked {
                run,
                reason,
                retry,
                marked,
            }) => {
                let state = if marked {
                    "is blocked"
                } else {
                    "could not be marked blocked (the server did not accept the update)"
                };
                Ok(BeginOutcome::Blocked(format!(
                    "Run {run} {state}: {}\nTo retry: {}\n",
                    shown(&reason),
                    shown(&retry)
                )))
            }
        }
    }

    fn render(
        parsed: &Options,
        outcome: BeginOutcome,
    ) -> Result<crate::cli::Output, Box<dyn std::error::Error>> {
        match outcome {
            BeginOutcome::Begun { begun, plan, next } => {
                if parsed.json {
                    let shown = Output {
                        begun: &begun,
                        plan,
                        next: next.as_ref(),
                    };
                    Ok(crate::cli::Output::json(&shown)?)
                } else {
                    let mut text = render(&begun, &plan);
                    if let Some(next) = &next {
                        text.push_str(&format!("\n{}", crate::run::next::render(next)));
                    }
                    Ok(crate::cli::Output::text(text))
                }
            }
            BeginOutcome::Blocked(message) => Ok(crate::cli::Output {
                stderr: message,
                exit_code: 1,
                ..crate::cli::Output::default()
            }),
        }
    }
}

/// `options` followed by the `--session` and `--agent-home` options of each agent session this command runs in.
///
/// They are `--session <id>` and `--agent-home <agent>=<path>`, so the update records the session and its token usage can
/// be read from the agent's own logs and attributed to the run.
pub(crate) fn with_sessions(options: &[impl AsRef<str>]) -> Vec<String> {
    let mut all: Vec<String> = options.iter().map(|o| o.as_ref().to_owned()).collect();
    for session in current_agent_sessions() {
        all.extend(session.update_options());
    }
    all
}

/// The agent sessions this command runs in, when the agent exposes them.
///
/// Each has the agent it belongs to and that agent's home folder: Claude Code sets `CLAUDE_CODE_SESSION_ID`; Codex sets `CODEX_SESSION_ID` (the
/// root session) and `CODEX_THREAD_ID`; Copilot CLI sets `COPILOT_AGENT_SESSION_ID`. A warning is printed
/// for a home that is not recorded because the dashboard would refuse it.
pub(crate) fn current_agent_sessions() -> Vec<AgentSession> {
    let (sessions, warnings) = agent_sessions_from(
        &|name| std::env::var_os(name),
        crate::install::home_dir().ok().as_deref(),
    );
    for warning in warnings {
        eprintln!("{warning}");
    }
    sessions
}

/// The agent sessions `var` names, each with the home folder `jidoka install` would use for its agent
/// (honoring `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `COPILOT_HOME`), and a warning for each agent home the
/// dashboard would refuse. Such a home is left out, so it cannot fail the command; a relative one, because
/// the dashboard cannot tell what it is relative to.
pub(crate) fn agent_sessions_from(
    var: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    user_home: Option<&std::path::Path>,
) -> (Vec<AgentSession>, Vec<String>) {
    let mut sessions: Vec<AgentSession> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for (name, agent) in [
        ("CLAUDE_CODE_SESSION_ID", Agent::Claude),
        ("CODEX_SESSION_ID", Agent::Codex),
        ("CODEX_THREAD_ID", Agent::Codex),
        ("COPILOT_AGENT_SESSION_ID", Agent::Copilot),
    ] {
        let Some(id) = var(name).and_then(|value| value.into_string().ok()) else {
            continue;
        };
        if crate::run::check_session_id(&id).is_err() || sessions.iter().any(|known| known.id == id)
        {
            continue;
        }
        let mut home = None;
        if let Some(agent_home) = agent.home_in(user_home, var) {
            match agent_home.into_os_string().into_string() {
                Ok(path) if crate::run::is_valid_agent_home(&path) => home = Some(path),
                _ => {
                    // Without the user's home, only a set variable can name the Agent home.
                    let source = if agent.home_in(None, var).is_some() {
                        agent.home_var()
                    } else {
                        "the home directory"
                    };
                    let agent = agent.name();
                    let warning = format!(
                        "Warning: the {agent} home from {source} is not an absolute path the dashboard accepts, \
                         so this run's {agent} usage is read from the dashboard's own {agent} home."
                    );
                    if !warnings.contains(&warning) {
                        warnings.push(warning);
                    }
                }
            }
        }
        sessions.push(AgentSession {
            id,
            agent: Some(agent),
            home,
        });
    }
    (sessions, warnings)
}

#[cfg(test)]
mod tests;
