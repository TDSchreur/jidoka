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

    /// Runs git in a folder. Tests override this to keep the user's own git settings out.
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        git(dir, args)
    }
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
    /// `running`, `blocked` or `done`. Only a running run is updated.
    pub(crate) status: String,
}

impl RunInfo {
    pub(crate) fn running(&self) -> bool {
        self.status == "running"
    }
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
    /// Whether the run now records the pull request; `false` for a run that is not running.
    pub(crate) recorded: bool,
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

/// An open pull request of the branch.
struct Existing {
    url: String,
    /// Its base branch, when `gh` said.
    base: Option<String>,
}

/// The first open pull request of `branch` in this repository. `gh pr list --head` matches by branch name
/// only, so a pull request from a fork that happens to use the same name is skipped.
fn open_pull_request(
    gh: &dyn Gh,
    dir: &Path,
    branch: &str,
) -> Result<Option<Existing>, ToolProblem> {
    let args = [
        "pr",
        "list",
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "url,baseRefName,isCrossRepository",
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
    let Some(item) = list
        .iter()
        .find(|item| item["isCrossRepository"].as_bool() != Some(true))
    else {
        return Ok(None);
    };
    let url = item["url"].as_str().ok_or_else(|| {
        ToolProblem(
            "`gh pr list` gave an open pull request without a URL, so it was not reused.".into(),
        )
    })?;
    Ok(Some(Existing {
        url: url.to_string(),
        base: item["baseRefName"]
            .as_str()
            .filter(|base| !base.is_empty())
            .map(str::to_string),
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

    let branch = gh
        .git(dir, &["branch", "--show-current"])
        .map_err(|error| {
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
    // A branch name that git could read as an option or a refspec (`+main` forces an update of main) is
    // never handed to git.
    if !valid_branch_name(&branch) {
        return Err(ToolProblem(format!(
            "The branch name {} is one this command will not push: it could be read as a git option or a forced refspec. Rename the branch (`git branch -m`) and try again.",
            shown(&branch)
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
        None => gh
            .git(
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
    let status = gh.git(dir, &["status", "--porcelain"]).map_err(|error| {
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
    let ahead = gh
        .git(
            dir,
            &["rev-list", "--count", &format!("refs/remotes/origin/{base}..HEAD")],
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
    let commit = gh.git(dir, &["rev-parse", "HEAD"]).map_err(|error| {
        ToolProblem(format!(
            "HEAD could not be read: {}",
            shown(first_line(&error))
        ))
    })?;

    // Look for the pull request before pushing: it also shows whether `gh` is signed in.
    let existing = open_pull_request(gh, dir, &branch)?;
    // The branch is pushed by its full name on both sides, so that no name is read as a refspec.
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    gh.git(dir, &["push", "-u", "origin", &refspec])
        .map_err(|error| push_problem(&branch, &error))?;

    let (pull_request, created, base) = match existing {
        Some(found) => (found.url, false, found.base.unwrap_or(base)),
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
                // Someone may have opened it between the look and now: use theirs.
                return match open_pull_request(gh, dir, &branch) {
                    Ok(Some(found)) => finish(
                        run,
                        api,
                        branch,
                        found.base.unwrap_or(base),
                        commit,
                        found.url,
                        false,
                    ),
                    _ => Err(problem("pr create", &output)),
                };
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
    finish(run, api, branch, base, commit, pull_request, created)
}

/// Records the pull request on a running run and returns the result.
fn finish(
    run: &RunInfo,
    api: &dyn RunApi,
    branch: String,
    base: String,
    commit: String,
    pull_request: String,
    created: bool,
) -> Result<Opened, ToolProblem> {
    if run.running() {
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
        recorded: run.running(),
    })
}

/// The result as text.
pub(crate) fn render(opened: &Opened, run: &RunInfo) -> String {
    let mut text = format!(
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
    );
    if !opened.recorded {
        text.push_str(&format!(
            "  The run was not updated (it is {}). Record the pull request with `factory run update {} {} {} --pr {}`.\n",
            shown(&run.status),
            shown(&run.id),
            shown(&run.stage),
            shown(&run.status),
            shown(&opened.pull_request)
        ));
    }
    text
}

#[cfg(test)]
#[path = "pr_open_tests.rs"]
mod tests;
