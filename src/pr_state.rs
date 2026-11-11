//! Pull request state: whether a Run's pull request is open, a draft, merged or closed, read from its host
//! (GitHub with `gh`, Azure Repos with `az`) and kept on the Run so the dashboard can show it.

use crate::pr_wait::{
    Gh, ToolProblem, azure_pr_head, azure_pr_status, is_azure_pr_url, problem, run_gh,
};
use crate::run::{FieldUpdate, Run, RunStatus, RunStore, Stage, UpdateRun};
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

impl PrState {
    /// How the state reads in a message.
    pub(crate) fn label(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Draft => "a draft",
            PrState::Merged => "merged",
            PrState::Closed => "closed",
            PrState::Unknown => "unknown",
        }
    }
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

/// The head of a pull request: its last commit and the branch it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Head {
    pub(crate) commit: String,
    pub(crate) branch: String,
}

/// Where a pull request's commits are read from.
enum Host {
    Azure,
    /// GitHub, with the pull request as `<repository>/pull/<number>`.
    GitHub(String),
}

/// Checks that `url` is an http(s) pull request of a host with commits and says which host.
fn host(url: &str) -> Result<Host, ToolProblem> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ToolProblem(format!(
            "The pull request is not an http(s) URL: {url}"
        )));
    }
    if is_azure_pr_url(url) {
        return Ok(Host::Azure);
    }
    github_pull_request(url).map(Host::GitHub).ok_or_else(|| {
        ToolProblem("Only GitHub and Azure Repos pull requests have a commit.".into())
    })
}

/// The head of the pull request at `url`, read in one call: GitHub's `headRefOid` and `headRefName`, or
/// Azure's last source commit and `sourceRefName`.
pub(crate) fn head(tool: &dyn Gh, url: &str) -> Result<Head, ToolProblem> {
    let (commit, branch) = match host(url)? {
        Host::Azure => azure_pr_head(tool, url)?,
        Host::GitHub(pull_request) => {
            let data = view_json(tool, &pull_request, "headRefOid,headRefName")?;
            let text = |field: &str| data[field].as_str().unwrap_or("").to_string();
            (text("headRefOid"), text("headRefName"))
        }
    };
    if branch.is_empty() {
        return Err(ToolProblem(
            "The host did not report the branch the pull request comes from.".into(),
        ));
    }
    Ok(Head {
        commit: valid_commit(commit)?,
        branch,
    })
}

fn valid_commit(commit: String) -> Result<String, ToolProblem> {
    if commit.len() < 4 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ToolProblem(
            "The host did not report a valid commit for the pull request.".into(),
        ));
    }
    Ok(commit.to_ascii_lowercase())
}

/// `gh pr view <pull request> --json <fields>`, parsed.
fn view_json(
    tool: &dyn Gh,
    pull_request: &str,
    fields: &str,
) -> Result<serde_json::Value, ToolProblem> {
    let output = run_gh(tool, &["pr", "view", pull_request, "--json", fields])?;
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

/// The reason a run is blocked when its pull request was closed without merging.
pub(crate) const CLOSED_REASON: &str = "PR closed without merge";

/// What a read state means for a run: a merged pull request finishes a run that is not done, and a closed one
/// blocks a running run. Both clear the activity, which was about waiting for a pull request that is gone.
/// Anything else, including an unreachable host, leaves the run as it is.
pub(crate) fn follow_up(run: &Run, state: PrState) -> Option<UpdateRun> {
    let mut update = match (state, run.status) {
        (PrState::Merged, RunStatus::Running | RunStatus::Blocked) => {
            let mut update = UpdateRun::new(Stage::Finish, RunStatus::Done);
            update.blocking_reason = FieldUpdate::Clear;
            // Finish cannot be done without a summary; keep the agent's when it wrote one.
            if !run.summaries.contains_key(&Stage::Finish) {
                update
                    .summaries
                    .insert(Stage::Finish, Some("Pull request merged".into()));
            }
            update
        }
        (PrState::Closed, RunStatus::Running) => {
            let mut update = UpdateRun::new(Stage::Finish, RunStatus::Blocked);
            update.blocking_reason = FieldUpdate::Set(CLOSED_REASON.into());
            update
        }
        _ => return None,
    };
    update.activity = FieldUpdate::Clear;
    Some(update)
}

/// Says what a read state is and, when `changed`, what became of the run.
pub(crate) fn describe(state: PrState, changed: bool) -> String {
    let mut text = format!("Pull request is {}.\n", state.label());
    if changed {
        text.push_str(match state {
            PrState::Merged => {
                "The run is finished. Run `jidoka run cleanup <id>` to remove its worktree.\n"
            }
            PrState::Closed => "The run is blocked: the pull request was closed without merging.\n",
            PrState::Open | PrState::Draft | PrState::Unknown => "",
        });
    }
    text
}

/// Reads the pull request of `run` from its host, stores the state and applies what it means for the run.
fn sync(store: &RunStore, tool: &dyn Gh, run: &Run, url: &str, now: u64) -> io::Result<PrStatus> {
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
    store.set_pr_status(&run.id, status.clone())?;
    // The host answer may take seconds; decide from the run as it is now, not as it was when asked.
    let current = store
        .list()?
        .into_iter()
        .find(|current| current.id == run.id);
    if let Some(update) = current.as_ref().and_then(|run| follow_up(run, state)) {
        store.update(&run.id, update)?;
    }
    Ok(status)
}

/// Syncs one run now, whatever its stored state says. `None` when the run does not exist or records no pull request.
pub(crate) fn sync_run(
    store: &RunStore,
    tool: &dyn Gh,
    id: &str,
    now: u64,
) -> io::Result<Option<PrStatus>> {
    let Some(run) = store.list()?.into_iter().find(|run| run.id == id) else {
        return Ok(None);
    };
    let Some(url) = run.pr.clone() else {
        return Ok(None);
    };
    sync(store, tool, &run, &url, now).map(Some)
}

/// One pass: syncs every Run whose pull request state is due. A problem is stored as unknown with its reason
/// and tried again on a later pass. Returns how many pull requests were read.
pub(crate) fn refresh(store: &RunStore, tool: &dyn Gh, now: u64) -> io::Result<usize> {
    let mut checked = 0;
    for run in store.list()? {
        let Some(url) = run.pr.as_deref() else {
            continue;
        };
        if !due(run.pr_status.as_ref(), url, now) {
            continue;
        }
        sync(store, tool, &run, url, now)?;
        checked += 1;
    }
    Ok(checked)
}

#[cfg(test)]
#[path = "pr_state_tests.rs"]
mod tests;
