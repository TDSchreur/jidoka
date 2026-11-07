//! Pull request state: whether a Run's pull request is open, a draft, merged or closed, read from its host
//! (GitHub with `gh`, Azure Repos with `az`) and kept on the Run so the dashboard can show it.

use crate::pr_wait::{azure_pr_status, is_azure_pr_url, problem, run_gh, Gh, ToolProblem};
use crate::run::RunStore;
use serde::{Deserialize, Serialize};
use std::io;

/// How long a state that can still change is trusted before the host is asked again.
pub(crate) const RECHECK_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
    /// The host could not be asked; the reason says why.
    Unknown,
}

/// The state of the pull request at `url`, as last read. The dashboard server owns it: callers cannot set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PrStatus {
    /// The pull request this state belongs to. A Run that records another pull request is read again.
    pub(crate) url: String,
    pub(crate) state: PrState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    /// When the host was asked, in milliseconds since the Unix epoch.
    pub(crate) checked_at: u64,
}

/// Reads the state of the pull request at `url` from its host.
pub(crate) fn read(tool: &dyn Gh, url: &str) -> Result<PrState, ToolProblem> {
    // Only an absolute http(s) URL is handed to a tool, so a stored value can never become an option.
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ToolProblem(format!(
            "The pull request is not an http(s) URL: {url}"
        )));
    }
    if is_azure_pr_url(url) {
        return match azure_pr_status(tool, url)? {
            (status, true) if status == "active" => Ok(PrState::Draft),
            (status, _) if status == "active" => Ok(PrState::Open),
            (status, _) if status == "completed" => Ok(PrState::Merged),
            (status, _) if status == "abandoned" => Ok(PrState::Closed),
            (status, _) => Err(ToolProblem(format!(
                "Azure reported an unknown pull request status: {status}"
            ))),
        };
    }
    let Some(pull_request) = github_pull_request(url) else {
        return Err(ToolProblem(
            "Only GitHub and Azure Repos pull requests have a state on the dashboard.".into(),
        ));
    };
    let output = run_gh(
        tool,
        &["pr", "view", &pull_request, "--json", "state,isDraft"],
    )?;
    if output.code != Some(0) {
        return Err(problem("pr view", &output));
    }
    let data: serde_json::Value = serde_json::from_str(&output.stdout).map_err(|error| {
        ToolProblem(format!(
            "`gh pr view` gave output that is not JSON: {error}"
        ))
    })?;
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

/// The GitHub (or GitHub Enterprise) pull request a link points to, as `<repository>/pull/<number>`: a link to
/// one of its tabs (`/files`) or comments (`#issuecomment-1`) is cut back to the pull request. None for a link
/// that is not a pull request.
fn github_pull_request(url: &str) -> Option<String> {
    let page = url.split(['?', '#']).next().unwrap_or(url);
    // The last /pull/<number>, so a repository that is itself called pull still works.
    page.rmatch_indices("/pull/").find_map(|(at, marker)| {
        let number = page[at + marker.len()..].split('/').next().unwrap_or("");
        (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("{}/pull/{number}", &page[..at]))
    })
}

/// Whether the stored state of a Run's pull request `url` should be read again at `now`.
fn due(status: Option<&PrStatus>, url: &str, now: u64) -> bool {
    match status {
        None => true,
        Some(status) if status.url != url => true,
        Some(status) if status.state == PrState::Merged => false,
        Some(status) => now.saturating_sub(status.checked_at) >= RECHECK_MS,
    }
}

/// One pass: reads the state of every Run's pull request that is due and stores it. A problem is stored as
/// unknown with its reason and tried again on a later pass. Returns how many pull requests were read.
pub(crate) fn refresh(store: &RunStore, tool: &dyn Gh, now: u64) -> io::Result<usize> {
    let mut checked = 0;
    for run in store.list()? {
        let Some(url) = run.pr.as_deref() else {
            continue;
        };
        if !due(run.pr_status.as_ref(), url, now) {
            continue;
        }
        let (state, reason) = match read(tool, url) {
            Ok(state) => (state, None),
            Err(ToolProblem(reason)) => (PrState::Unknown, Some(reason)),
        };
        let status = PrStatus {
            url: url.into(),
            state,
            reason,
            checked_at: now,
        };
        store.set_pr_status(&run.id, status)?;
        checked += 1;
    }
    Ok(checked)
}

#[cfg(test)]
#[path = "pr_state_tests.rs"]
mod tests;
