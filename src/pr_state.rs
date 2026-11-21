//! Pull request state: whether a Run's pull request is open, a draft, merged or closed, read from its
//! Pull request host and kept on the Run so the dashboard can show it.

use crate::pr_host::{PrHost, ToolProblem};
use crate::run::{LifecycleSource, PrResult, Run, RunStatus, RunStore, Stage};
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
    /// How final the state is: on an equal check time only a higher rank replaces a recorded state.
    fn rank(self) -> u8 {
        match self {
            PrState::Merged => 2,
            PrState::Closed => 1,
            PrState::Open | PrState::Draft | PrState::Unknown => 0,
        }
    }

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

/// The state of the pull request at `url`, as last read. The dashboard records host results, including
/// results supplied by authenticated CLI clients, together with any resulting run transition.
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

impl PrStatus {
    /// Whether this state may replace `previous`: the one Pull request state rule.
    ///
    /// A state of another pull request always does; a merged state is never overwritten; an older check
    /// loses; on an equal check time only the same or a higher-ranked state (merged, then closed, then the
    /// rest) replaces.
    pub(crate) fn supersedes(&self, previous: &PrStatus) -> bool {
        if self.url != previous.url {
            return true;
        }
        if previous.state == PrState::Merged && self.state != PrState::Merged {
            return false;
        }
        match self.checked_at.cmp(&previous.checked_at) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => {
                self.state == previous.state || self.state.rank() > previous.state.rank()
            }
        }
    }
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

/// Applies the lifecycle consequence of an observed merge or close.
pub(crate) fn follow_up(run: &mut Run, status: &PrStatus, at: u64) -> Result<bool, String> {
    match (status.state, run.status) {
        (PrState::Merged, RunStatus::Running | RunStatus::Blocked)
            if run.stage == Stage::Finish =>
        {
            run.leave(
                crate::run::Exit::Merged {
                    url: status.url.clone(),
                },
                at,
            )?;
        }
        (PrState::Closed, RunStatus::Running) => {
            run.block(CLOSED_REASON, LifecycleSource::PullRequest, at)?;
            run.activity = None;
        }
        _ => return Ok(false),
    }
    Ok(true)
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
fn sync(
    store: &RunStore,
    host: &dyn PrHost,
    run: &Run,
    url: &str,
    now: u64,
) -> io::Result<(PrStatus, PrResult)> {
    let (state, reason) = match host.state(url) {
        Ok(state) => (state, None),
        Err(ToolProblem(reason)) => (PrState::Unknown, Some(reason)),
    };
    let status = PrStatus {
        url: url.into(),
        state,
        reason,
        checked_at: now,
    };
    let result = store.record_pr_result(&run.id, status.clone())?;
    Ok((status, result))
}

/// Syncs one run now, whatever its stored state says. `None` when the run does not exist or records no pull request.
pub(crate) fn sync_run(
    store: &RunStore,
    host: &dyn PrHost,
    id: &str,
    now: u64,
) -> io::Result<Option<(PrStatus, PrResult)>> {
    let Some(run) = store.list()?.into_iter().find(|run| run.id == id) else {
        return Ok(None);
    };
    let Some(url) = run.pr.clone() else {
        return Ok(None);
    };
    sync(store, host, &run, &url, now).map(Some)
}

/// One pass: syncs every Run whose pull request state is due. A problem is stored as unknown with its reason
/// and tried again on a later pass. Returns how many pull requests were read.
#[cfg(test)]
pub(crate) fn refresh(store: &RunStore, host: &dyn PrHost, now: u64) -> io::Result<usize> {
    refresh_with_clock(store, host, || Ok(now))
}

pub(crate) fn refresh_live(store: &RunStore, host: &dyn PrHost) -> io::Result<usize> {
    refresh_with_clock(store, host, crate::run::updated_now)
}

fn refresh_with_clock(
    store: &RunStore,
    host: &dyn PrHost,
    clock: impl Fn() -> io::Result<u64>,
) -> io::Result<usize> {
    let mut checked = 0;
    for run in store.list()? {
        let Some(url) = run.pr.as_deref() else {
            continue;
        };
        let now = clock()?;
        if !due(run.pr_status.as_ref(), url, now) {
            continue;
        }
        sync(store, host, &run, url, now)?;
        checked += 1;
    }
    Ok(checked)
}

#[cfg(test)]
#[path = "pr_state_tests.rs"]
mod tests;
