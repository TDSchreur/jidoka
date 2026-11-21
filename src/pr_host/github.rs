//! The GitHub adapter: reads pull requests on GitHub and GitHub Enterprise with `gh`.

use super::{Adapter, Head, ToolError, ToolOutput, ToolProblem, Tools, unsupported};
use crate::{inspect::shown, pr_state::PrState};
use std::path::Path;

pub(super) struct GitHub;

impl Adapter for GitHub {
    fn state(&self, tools: &dyn Tools, url: &str) -> Result<PrState, ToolProblem> {
        state_of(&view(tools, url, "state,isDraft")?)
    }

    fn head(&self, tools: &dyn Tools, url: &str) -> Result<Head, ToolProblem> {
        let data = view(tools, url, "state,isDraft,headRefOid,headRefName")?;
        let text = |field: &str| data[field].as_str().unwrap_or("").to_string();
        Ok(Head {
            state: state_of(&data)?,
            commit: text("headRefOid"),
            branch: text("headRefName"),
        })
    }
}

fn state_of(data: &serde_json::Value) -> Result<PrState, ToolProblem> {
    match (data["state"].as_str(), data["isDraft"].as_bool()) {
        (Some("OPEN"), Some(true)) => Ok(PrState::Draft),
        (Some("OPEN"), _) => Ok(PrState::Open),
        (Some("MERGED"), _) => Ok(PrState::Merged),
        (Some("CLOSED"), _) => Ok(PrState::Closed),
        (other, _) => Err(ToolProblem(format!(
            "GitHub reported an unknown pull request state: {}",
            other.unwrap_or("none")
        ))),
    }
}

/// `gh pr view <pull request> --json <fields>`, parsed.
fn view(tools: &dyn Tools, url: &str, fields: &str) -> Result<serde_json::Value, ToolProblem> {
    let pull_request = pull_request(url).ok_or_else(unsupported)?;
    let output = run_gh(
        tools,
        None,
        &["pr", "view", &pull_request, "--json", fields],
    )?;
    if output.code != Some(0) {
        return Err(problem("pr view", &output));
    }
    serde_json::from_str(&output.stdout).map_err(|error| {
        ToolProblem(format!(
            "`gh pr view` gave output that is not JSON: {error}"
        ))
    })
}

/// The GitHub (or GitHub Enterprise) pull request a link points to, as `<repository>/pull/<number>`: a link to
/// one of its tabs (`/files`) or comments (`#issuecomment-1`) is cut back to the pull request. None for a link
/// that is not a pull request.
pub(super) fn pull_request(url: &str) -> Option<String> {
    let page = url.split(['?', '#']).next().unwrap_or(url);
    // The last /pull/<number>, so a repository that is itself called pull still works.
    page.rmatch_indices("/pull/").find_map(|(at, marker)| {
        let number = page[at + marker.len()..].split('/').next().unwrap_or("");
        (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("{}/pull/{number}", &page[..at]))
    })
}

/// Runs `gh` with `args`, in `dir` when given.
///
/// # Errors
///
/// Returns a problem that says what to do when `gh` cannot be started.
pub(crate) fn run_gh(
    tools: &dyn Tools,
    dir: Option<&Path>,
    args: &[&str],
) -> Result<ToolOutput, ToolProblem> {
    started(tools.gh(dir, args))
}

/// What `gh` printed, or a problem that says what to do when it could not be started.
///
/// # Errors
///
/// Returns the problem when `result` says `gh` is not installed or did not start.
pub(crate) fn started(result: Result<ToolOutput, ToolError>) -> Result<ToolOutput, ToolProblem> {
    result.map_err(|error| match error {
        ToolError::NotInstalled => ToolProblem(
            "The GitHub CLI (`gh`) is not installed or not on PATH. Install it from https://cli.github.com, run `gh auth login`, and try again."
                .into(),
        ),
        ToolError::Other(text) => ToolProblem(format!("Could not run `gh`: {text}")),
    })
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
}

/// A failed `gh <what>` call as a message that says what to do.
pub(crate) fn problem(what: &str, output: &ToolOutput) -> ToolProblem {
    let stderr = output.stderr.to_lowercase();
    if output.code == Some(4) || stderr.contains("gh auth login") {
        return ToolProblem(
            "The GitHub CLI is not signed in. Run `gh auth login` and try again.".into(),
        );
    }
    let detail = shown(first_line(if output.stderr.trim().is_empty() {
        &output.stdout
    } else {
        &output.stderr
    }));
    if stderr.contains("error connecting")
        || stderr.contains("could not resolve host")
        || stderr.contains("no such host")
        || stderr.contains("i/o timeout")
    {
        return ToolProblem(format!(
            "Could not reach GitHub. Check your network connection and try again. ({detail})"
        ));
    }
    ToolProblem(format!("`gh {what}` failed: {detail}"))
}
