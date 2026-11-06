//! `factory run begin`: creates a run and its workspace (a worktree on a new branch from the base) and records it.

use crate::{inspect::shown, routing::ModelRouting};
use serde::Serialize;
use std::{path::Path, process::Command};

/// The factory server, as far as this command needs it. Tests answer from a script.
pub(crate) trait RunApi {
    /// Creates a run and returns its id.
    fn create(
        &self,
        title: &str,
        project: &str,
        repository: Option<String>,
        session: Option<String>,
        routing: Option<ModelRouting>,
    ) -> Result<String, String>;
    /// Reads a run that exists.
    fn get(&self, id: &str) -> Result<RunSummary, String>;
    /// Updates a run like `factory run update <id> <stage> <status> <options>`.
    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String>;
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
    pub(crate) agent: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) base: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) json: bool,
}

/// Whether `name` can be a branch name that is safe to pass to git: no option look-alike, no spaces or
/// control characters, none of the characters git refuses, and no `..`, `@{`, `.lock` ending or edge slash.
fn valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && name != "@"
        && !name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
}

/// The options of `factory run begin --run <id>`: the same, without a title.
pub(crate) fn parse_resume(run: &str, options: &[&str]) -> Result<Options, String> {
    if run.is_empty() || !run.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
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
                if crate::routing::Host::parse(name).is_none() {
                    return Err(format!(
                        "Unknown agent {name:?}; use codex, claude, or copilot."
                    ));
                }
                parsed.agent = Some(name.to_string());
            }
            "--mode" if parsed.mode.is_none() => {
                let mode = value("--mode")?;
                serde_json::from_value::<crate::run::Mode>(serde_json::Value::from(mode)).map_err(
                    |_| {
                        format!(
                            "Invalid value for --mode: {mode}; use factory-full, factory-balanced or factory-fast"
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
/// The longest branch name derived from a title.
const MAX_NAME_CHARS: usize = 40;

/// Runs git in `dir`. The error is what git printed, or why it could not be started.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        // Its messages are told apart below, so they must be in English.
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .map_err(|error| format!("git could not be started: {error}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    Err(text.trim().to_string())
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
}

/// The last line git printed, where its `fatal:` message is (progress lines come first).
fn last_line(text: &str) -> &str {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
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
    let mut command = format!("factory run begin --run {run}");
    for (flag, value) in [
        ("--agent", &options.agent),
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

/// Creates the run and its workspace. Never deletes, resets, checks out over or cleans anything.
pub(crate) fn begin(
    dir: &Path,
    options: &Options,
    api: &dyn RunApi,
    session: Option<String>,
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
                    session.clone(),
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
            if let Some(session) = &session {
                args.extend(["--session", session]);
            }
            match api.update(&run, "define", "running", &args) {
                Ok(()) => Ok(Begun {
                    run,
                    worktree: workspace.worktree,
                    branch: workspace.branch,
                    base: workspace.base,
                    base_commit: workspace.base_commit,
                }),
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
    let marked = api
        .update(run, "define", "blocked", &["--blocking-reason", &reason])
        .is_ok();
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
    git(
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
    let list = git(main, &["worktree", "list", "--porcelain"]).ok()?;
    let wanted = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for block in list.split("\n\n") {
        let mut location = None;
        let mut branch = None;
        for line in block.lines() {
            if let Some(found) = line.strip_prefix("worktree ") {
                location = Some(Path::new(found).to_path_buf());
            } else if let Some(name) = line.strip_prefix("branch refs/heads/") {
                branch = Some(name.to_string());
            }
        }
        let location = location?;
        let same = location.canonicalize().unwrap_or_else(|_| location.clone()) == wanted
            || location == path;
        if same {
            return Some(branch);
        }
    }
    None
}

/// Whether `branch` has no commits of its own: everything on it is already on `origin/<base>`.
fn is_unused(main: &Path, branch: &str, base: &str) -> bool {
    git(
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
    git(main, &args).map(|_| ()).map_err(|error| {
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
fn workspace(
    main: &Path,
    options: &Options,
    title: &str,
    run: &str,
    resume: bool,
) -> Result<Workspace, Failure> {
    git(main, &["remote", "get-url", "origin"]).map_err(|_| {
        "This repository has no remote named origin. Add one with `git remote add origin <url>` and retry."
            .to_string()
    })?;
    let base = match &options.base {
        Some(base) => base.clone(),
        None => git(main, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .ok()
            .and_then(|head| head.strip_prefix("origin/").map(str::to_string))
            .ok_or_else(|| {
                "origin/HEAD is not set, so the base branch is not known. Pass it with --base, or run `git remote set-head origin -a`."
                    .to_string()
            })?,
    };
    // An explicit refspec also works in a clone that was made for one branch only.
    let refspec = format!("+refs/heads/{base}:refs/remotes/origin/{base}");
    git(main, &["fetch", "origin", &refspec]).map_err(|error| {
        if error.contains("couldn't find remote ref") {
            format!("The base branch {base} does not exist on origin. Pass an existing branch with --base.")
        } else {
            format!("Fetching {base} from origin failed: {}", first_line(&error))
        }
    })?;
    let base_commit = git(
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

    let parent = main.parent().unwrap_or(main);
    let repo_name = main
        .file_name()
        .map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
    let folder = parent.join(format!("{repo_name}.worktrees"));
    let path_of = |branch: &str| folder.join(branch.replace('/', "-"));
    let taken = |branch: &str| branch_exists(main, branch) || path_of(branch).exists();
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
                if number > 99 {
                    return Err(
                        format!("Every name from {derived} to {derived}-99 is taken.").into(),
                    );
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
        git(
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
            let status = git(&path, &["status", "--porcelain"])
                .map_err(|error| fail(format!("{path_text} could not be read: {}", first_line(&error))))?;
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
pub(crate) fn render(begun: &Begun) -> String {
    format!(
        "Run: {}\nWorktree: {}\nBranch: {}\nBase: {}\nBase commit: {}\n",
        shown(&begun.run),
        shown(&begun.worktree),
        shown(&begun.branch),
        shown(&begun.base),
        shown(&begun.base_commit)
    )
}

#[cfg(test)]
#[path = "run_begin_tests.rs"]
mod tests;
