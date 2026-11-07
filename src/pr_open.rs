//! `factory pr open`: pushes a run's branch and opens its pull request, or finds the one that is open.

use crate::{
    inspect::shown,
    pr_wait::{problem, GhError, GhOutput, ToolProblem},
    run_begin::{first_line, git, last_line, valid_branch_name},
};
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

/// Runs `gh` in a folder, so that it finds the repository from the git remotes there. Tests answer from a
/// script.
pub(crate) trait Gh {
    fn run(&self, dir: &Path, args: &[&str]) -> Result<GhOutput, GhError>;
}

/// The real `gh`.
pub(crate) struct RealGh;

impl Gh for RealGh {
    fn run(&self, dir: &Path, args: &[&str]) -> Result<GhOutput, GhError> {
        let output = Command::new("gh")
            .current_dir(dir)
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .output()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    GhError::NotInstalled
                } else {
                    GhError::Other(error.to_string())
                }
            })?;
        Ok(GhOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// The factory server, as far as this command needs it.
pub(crate) trait RunApi {
    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String>;
}

/// What the command needs to know about the run.
#[derive(Clone, Debug)]
pub(crate) struct RunInfo {
    pub(crate) id: String,
    pub(crate) worktree: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) stage: String,
    /// Only a running run is updated.
    pub(crate) running: bool,
}

/// The longest title and body file that are accepted.
const MAX_TITLE_CHARS: usize = 256;
const MAX_BODY_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub(crate) title: String,
    pub(crate) body_file: PathBuf,
    pub(crate) base: Option<String>,
    pub(crate) json: bool,
}

/// `--title <title> --body-file <path> [--base <branch>] [--json]`, each at most once.
pub(crate) fn parse_options(options: &[&str]) -> Result<Options, String> {
    let (mut title, mut body, mut base, mut json) = (None, None, None, false);
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .copied()
                .ok_or_else(|| format!("{flag} needs a value."))
        };
        match *arg {
            "--json" if !json => json = true,
            "--title" if title.is_none() => title = Some(value("--title")?.to_string()),
            "--body-file" if body.is_none() => body = Some(value("--body-file")?.to_string()),
            "--base" if base.is_none() => {
                let name = value("--base")?;
                if !valid_branch_name(name) {
                    return Err(format!("{name:?} is not a usable branch name for --base."));
                }
                base = Some(name.to_string());
            }
            "--json" | "--title" | "--body-file" | "--base" => {
                return Err(format!("{arg} was given twice."))
            }
            other => return Err(format!("Unexpected option {other:?}.")),
        }
    }
    let title = title.ok_or("--title is needed.")?;
    let body = body.ok_or("--body-file is needed.")?;
    if title.trim().is_empty()
        || title.chars().count() > MAX_TITLE_CHARS
        || title.chars().any(char::is_control)
    {
        return Err(format!(
            "The title must be one line of 1 to {MAX_TITLE_CHARS} characters."
        ));
    }
    let metadata = std::fs::metadata(&body)
        .map_err(|error| format!("The body file {body:?} cannot be read: {error}."))?;
    if !metadata.is_file() || metadata.len() > MAX_BODY_BYTES {
        return Err(format!(
            "The body file {body:?} must be a file of at most {} KiB.",
            MAX_BODY_BYTES / 1024
        ));
    }
    let body_file = std::fs::canonicalize(&body)
        .map_err(|error| format!("The body file {body:?} cannot be read: {error}."))?;
    Ok(Options {
        title,
        body_file,
        base,
        json,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Opened {
    pub(crate) pull_request: String,
    /// Whether this call opened it; `false` when it was open already.
    pub(crate) created: bool,
    pub(crate) branch: String,
    pub(crate) base: String,
    /// The commit that was pushed.
    pub(crate) commit: String,
}

fn run_gh(gh: &dyn Gh, dir: &Path, args: &[&str]) -> Result<GhOutput, ToolProblem> {
    gh.run(dir, args).map_err(|error| match error {
        GhError::NotInstalled => ToolProblem(
            "The GitHub CLI (`gh`) is not installed or not on PATH. Install it from https://cli.github.com, run `gh auth login`, and try again."
                .into(),
        ),
        GhError::Other(text) => ToolProblem(format!("Could not run `gh`: {text}")),
    })
}

/// A rejected or failed push as a message that says what to do.
fn push_problem(branch: &str, error: &str) -> ToolProblem {
    let lower = error.to_lowercase();
    let detail = shown(last_line(error));
    if lower.contains("rejected")
        || lower.contains("non-fast-forward")
        || lower.contains("fetch first")
    {
        return ToolProblem(format!(
            "origin rejected the push of {branch}: it has commits this branch does not. Merge the remote branch or the base into it with a normal merge, never a force-push, and try again. ({detail})"
        ));
    }
    if lower.contains("authentication failed")
        || lower.contains("could not read username")
        || lower.contains("permission denied")
        || lower.contains("publickey")
    {
        return ToolProblem(format!(
            "git is not allowed to push to origin. Sign in (for example `gh auth login`, or set up your SSH key or credential helper) and try again. ({detail})"
        ));
    }
    if lower.contains("could not resolve host") || lower.contains("unable to access") {
        return ToolProblem(format!(
            "Could not reach origin. Check your network connection and try again. ({detail})"
        ));
    }
    ToolProblem(format!("Pushing {branch} to origin failed: {detail}"))
}

/// The first open pull request of `branch`, as its URL and base branch.
fn open_pull_request(
    gh: &dyn Gh,
    dir: &Path,
    branch: &str,
) -> Result<Option<(String, String)>, ToolProblem> {
    let args = [
        "pr",
        "list",
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "url,baseRefName",
    ];
    let output = run_gh(gh, dir, &args)?;
    if output.code != Some(0) {
        return Err(problem("pr list", &output));
    }
    let list: Vec<serde_json::Value> =
        serde_json::from_str(output.stdout.trim()).map_err(|error| {
            ToolProblem(format!(
                "`gh pr list` gave output that is not JSON: {error}"
            ))
        })?;
    Ok(list.first().and_then(|item| {
        Some((
            item["url"].as_str()?.to_string(),
            item["baseRefName"].as_str().unwrap_or("").to_string(),
        ))
    }))
}

/// Pushes the run's branch and opens its pull request, or finds the open one, and records its URL on the run.
/// Nothing is pushed unless the checks pass and `gh` can see the repository. It never forces a push.
pub(crate) fn open(
    run: &RunInfo,
    options: &Options,
    gh: &dyn Gh,
    api: &dyn RunApi,
) -> Result<Opened, ToolProblem> {
    let worktree = run.worktree.as_deref().ok_or_else(|| {
        ToolProblem(format!(
            "Run {} has no worktree recorded. Record it with `factory run update` (--worktree and --branch), or create the workspace with `factory run begin`.",
            run.id
        ))
    })?;
    let dir = Path::new(worktree);
    if !dir.is_dir() {
        return Err(ToolProblem(format!(
            "The recorded worktree {} does not exist.",
            shown(worktree)
        )));
    }
    run_gh(gh, dir, &["--version"])?;

    let branch = git(dir, &["branch", "--show-current"]).map_err(|error| {
        ToolProblem(format!(
            "{} could not be read: {}",
            shown(worktree),
            shown(first_line(&error))
        ))
    })?;
    if branch.is_empty() {
        return Err(ToolProblem(format!(
            "The worktree {} is not on a branch (a detached HEAD).",
            shown(worktree)
        )));
    }
    if let Some(recorded) = &run.branch {
        if *recorded != branch {
            return Err(ToolProblem(format!(
                "The worktree is on {} but the run records {}. Switch it back, or record the right branch with `factory run update`.",
                shown(&branch),
                shown(recorded)
            )));
        }
    }
    let base = match &options.base {
        Some(base) => base.clone(),
        None => git(
            dir,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .ok()
        .and_then(|head| head.strip_prefix("origin/").map(str::to_string))
        .ok_or_else(|| {
            ToolProblem(
                "origin/HEAD is not set, so the base branch is not known. Pass it with --base."
                    .into(),
            )
        })?,
    };
    if branch == base {
        return Err(ToolProblem(format!(
            "{} is the base branch. A pull request needs a feature branch.",
            shown(&branch)
        )));
    }
    let status = git(dir, &["status", "--porcelain"]).map_err(|error| {
        ToolProblem(format!(
            "The status of {} could not be read: {}",
            shown(worktree),
            shown(first_line(&error))
        ))
    })?;
    if !status.is_empty() {
        return Err(ToolProblem(format!(
            "The worktree has uncommitted changes ({} paths). Commit them, then try again.",
            status.lines().count()
        )));
    }
    let ahead = git(
        dir,
        &[
            "rev-list",
            "--count",
            &format!("refs/remotes/origin/{base}..HEAD"),
        ],
    )
    .map_err(|_| {
        ToolProblem(format!(
            "origin/{} is not known in this repository. Run `git fetch origin {}` and try again.",
            shown(&base),
            shown(&base)
        ))
    })?;
    if ahead == "0" {
        return Err(ToolProblem(format!(
            "{} has no commits beyond origin/{}, so there is nothing to open a pull request for.",
            shown(&branch),
            shown(&base)
        )));
    }
    let commit = git(dir, &["rev-parse", "HEAD"]).map_err(|error| {
        ToolProblem(format!(
            "HEAD could not be read: {}",
            shown(first_line(&error))
        ))
    })?;

    // Look for the pull request before pushing: it also shows whether `gh` is signed in.
    let existing = open_pull_request(gh, dir, &branch)?;
    git(dir, &["push", "-u", "origin", &branch]).map_err(|error| push_problem(&branch, &error))?;

    let (pull_request, created, base) = match existing {
        Some((url, existing_base)) => (
            url,
            false,
            if existing_base.is_empty() {
                base
            } else {
                existing_base
            },
        ),
        None => {
            let body = options.body_file.display().to_string();
            let output = run_gh(
                gh,
                dir,
                &[
                    "pr",
                    "create",
                    "--base",
                    &base,
                    "--head",
                    &branch,
                    "--title",
                    &options.title,
                    "--body-file",
                    &body,
                ],
            )?;
            if output.code != Some(0) {
                return Err(problem("pr create", &output));
            }
            let url = output
                .stdout
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| line.starts_with("https://") || line.starts_with("http://"))
                .ok_or_else(|| {
                    ToolProblem(format!(
                        "`gh pr create` did not print the pull request's URL: {}",
                        shown(first_line(&output.stdout))
                    ))
                })?
                .to_string();
            (url, true, base)
        }
    };
    if run.running {
        api.update(
            &run.id,
            &run.stage,
            "running",
            &["--pr", &pull_request, "--activity", "Opened pull request"],
        )
        .map_err(|error| {
            ToolProblem(format!(
                "The pull request is open at {}, but the run could not be updated: {error}. Run the command again; it finds the pull request and records it.",
                shown(&pull_request)
            ))
        })?;
    }
    Ok(Opened {
        pull_request,
        created,
        branch,
        base,
        commit,
    })
}

/// The result as text.
pub(crate) fn render(opened: &Opened) -> String {
    format!(
        "{}: {}\n  Branch: {}\n  Base: {}\n  Pushed commit: {}\n",
        if opened.created {
            "Pull request opened"
        } else {
            "Pull request already open"
        },
        shown(&opened.pull_request),
        shown(&opened.branch),
        shown(&opened.base),
        shown(&opened.commit.chars().take(7).collect::<String>())
    )
}

#[cfg(test)]
#[path = "pr_open_tests.rs"]
mod tests;
