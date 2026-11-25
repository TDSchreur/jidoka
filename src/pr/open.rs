//! `jidoka pr open`: pushes a run's branch and opens its pull request, or finds the one that is open.

use crate::{
    git::{Git, first_line, valid_branch_name},
    pr::host::{NewPr, PrHost, ToolProblem, check_origin, detect_host, push_problem},
    pr::workspace::{PrReport, PrRun, RunReporter, Workspace, push_branch},
    run::inspect::shown,
};
use serde::Serialize;
use std::path::PathBuf;

/// A title longer than this is rejected, which keeps it a short one-line heading.
const MAX_TITLE_CHARS: usize = 256;
/// A body file larger than this is rejected, so the whole file is never read into memory.
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
                return Err(format!("{arg} was given twice."));
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

/// Pushes the run's branch and opens its pull request, or finds the open one, and records its URL on the run.
/// Nothing is pushed unless the checks pass and the host can see the repository. It never forces a push.
pub(crate) fn open(
    run: &PrRun,
    options: &Options,
    host: &dyn PrHost,
    git: &dyn Git,
    reporter: &dyn RunReporter,
) -> Result<Opened, ToolProblem> {
    let Workspace { dir, branch, base } = Workspace::verify(run, options.base.as_deref(), git)?;
    let dir = dir.as_path();
    let read = |args: &[&str]| git.git(dir, args).ok();
    let origin = detect_host(&read)?;
    check_origin(&origin, &read)?;

    let ahead = git
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
    let commit = git.git(dir, &["rev-parse", "HEAD"]).map_err(|error| {
        ToolProblem(format!(
            "HEAD could not be read: {}",
            shown(first_line(&error))
        ))
    })?;

    // Look for the pull request before pushing: it also shows whether the host's tool is signed in.
    let existing = host.find_open(dir, &origin, &branch)?;
    let new = NewPr {
        branch: &branch,
        base: &base,
        title: &options.title,
        body_file: &options.body_file,
    };
    if existing.is_none() {
        host.check_new(&origin, &new)?;
    }
    push_branch(git, dir, &branch).map_err(|error| push_problem(&origin, &branch, &error))?;

    let found = match existing {
        Some(found) => found,
        None => match host.create(dir, &origin, &new) {
            Ok(url) => return finish(run, reporter, branch, base, commit, url, true),
            // Someone may have opened it between the look and now: use theirs.
            Err(problem) => match host.find_open(dir, &origin, &branch) {
                Ok(Some(found)) => found,
                _ => return Err(problem),
            },
        },
    };
    let base = found.base.unwrap_or(base);
    finish(run, reporter, branch, base, commit, found.url, false)
}

/// Records the pull request on a running run and returns the result.
#[expect(
    clippy::too_many_arguments,
    reason = "seven inputs describing the opened PR; an Opened-like struct would collapse these"
)]
fn finish(
    run: &PrRun,
    reporter: &dyn RunReporter,
    branch: String,
    base: String,
    commit: String,
    pull_request: String,
    created: bool,
) -> Result<Opened, ToolProblem> {
    if run.running() {
        let report = PrReport::Opened {
            pull_request: pull_request.clone(),
        };
        reporter.report(&run.id, &report).map_err(|error| {
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
pub(crate) fn render(opened: &Opened, run: &PrRun) -> String {
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
            "  Run status is {}. Record the pull request with `jidoka run update {} --pr {}`.\n",
            shown(&run.status),
            shown(&run.id),
            shown(&opened.pull_request)
        ));
    }
    text
}

/// `jidoka pr open <run-id> --title <title> --body-file <path>`: opens the run's pull request.
///
/// It pushes the run's branch, opens the pull request or finds the one that is open, and records it on the run.
pub(crate) struct OpenCommand;

impl crate::cli::Command for OpenCommand {
    type Options = (String, Options);
    /// The pull request, and the run it belongs to.
    type Outcome = (Opened, PrRun);

    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn std::error::Error>> {
        let (id, options) = crate::cli::split_first(words);
        let parsed = parse_options(options).map_err(crate::cli::usage_error)?;
        crate::client::check_run_id(id)?;
        Ok((id.to_owned(), parsed))
    }

    fn run((id, parsed): &Self::Options) -> Result<Self::Outcome, Box<dyn std::error::Error>> {
        let run = crate::client::get_run(id)?;
        let info = PrRun::from_run(id, &run);
        let opened = open(
            &info,
            parsed,
            &crate::pr::host::real_hosts(),
            &crate::git::RealGit,
            &crate::client::ServerApi,
        )
        .map_err(|problem| shown(&problem.0))?;
        Ok((opened, info))
    }

    fn render(
        (_, parsed): &Self::Options,
        (opened, info): Self::Outcome,
    ) -> Result<crate::cli::Output, Box<dyn std::error::Error>> {
        Ok(crate::cli::Output::json_or_text(
            parsed.json,
            &opened,
            || render(&opened, &info),
        )?)
    }
}

#[cfg(test)]
mod tests;
