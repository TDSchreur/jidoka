//! Watching the checks of a pull request, the same way on every host: each adapter reads one consistent
//! snapshot of the pull request per poll, and the loop here decides the CI outcome from those snapshots.

use super::{Adapter, ToolProblem, Tools};
use crate::{
    pr::state::PrState,
    pr::wait::{BaseState, CheckResult, Clock, Options, Outcome, Waited, activity_text},
};
use std::time::Duration;

/// How long checks may take to appear before the pull request counts as having no CI.
const GRACE: Duration = Duration::from_secs(90);
/// How long the pull request may take to show a commit that was just pushed, and how often it is asked.
const HEAD_GRACE: Duration = Duration::from_secs(30);
const HEAD_POLL: Duration = Duration::from_secs(5);
/// How often the checks are asked for.
const POLL: Duration = Duration::from_secs(15);
/// How long to wait before asking again when the host has not worked out the base state yet.
const BASE_RETRY: Duration = Duration::from_secs(3);
/// How many more times an unknown base state is asked for.
const BASE_RETRIES: usize = 5;
/// How long to wait before asking again when the host failed to answer, and how many more times it is asked.
/// The retries stay within `--timeout`.
const SNAPSHOT_RETRY: Duration = Duration::from_secs(10);
const SNAPSHOT_RETRIES: usize = 3;

/// One look at a pull request while its checks are watched. The adapter reads the state and the head
/// before and after the checks, so the checks belong to the head it reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) state: PrState,
    /// The head commit.
    pub(super) head: String,
    /// The checks of the head; none when the pull request is not open.
    pub(super) checks: Vec<CheckResult>,
    /// Whether any check applies to the pull request. Without one it has no CI, once checks had time to
    /// appear.
    pub(super) applies: bool,
    /// Whether the branch is behind or conflicts with its base; `None` while the host is still working it
    /// out, so it is asked for again.
    pub(super) base: Option<BaseState>,
}

/// The pull request at `url`, read through its host's adapter with `tools`.
pub(super) struct PullRequest<'a> {
    pub(super) adapter: &'a dyn Adapter,
    pub(super) tools: &'a dyn Tools,
    pub(super) url: &'a str,
}

impl PullRequest<'_> {
    fn snapshot(&self) -> Result<Snapshot, ToolProblem> {
        self.adapter.snapshot(self.tools, self.url)
    }
}

/// Whether the checks of a pull request in `state` are watched: a draft has CI too.
fn is_open(state: PrState) -> bool {
    matches!(state, PrState::Open | PrState::Draft)
}

/// Watches the checks of `pr` for its current head commit until they have ended, and
/// returns the CI outcome with the state the pull request had. `activity` is told what the run is doing,
/// and `progress` how far the checks are. It only reads: it never merges, pushes or updates the branch.
pub(super) fn watch(
    pr: &PullRequest,
    options: &Options,
    clock: &dyn Clock,
    activity: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(&str),
) -> Result<Waited, ToolProblem> {
    // The whole wait, including the grace for a just-pushed head, stays within `--timeout`.
    let started = clock.elapsed();
    let deadline = started + options.timeout;
    let mut first = look(pr, clock, deadline, progress)?;
    let waited_for = || clock.elapsed().saturating_sub(started);
    if let Some(pushed) = &options.pushed_head {
        while is_open(first.state)
            && first.head != *pushed
            && waited_for() < HEAD_GRACE.min(options.timeout)
        {
            progress("Waiting for the pull request to show the pushed commit");
            clock.sleep(HEAD_POLL);
            first = look(pr, clock, deadline, progress)?;
        }
    }
    let head = first.head.clone();
    let ended = |outcome: Outcome, look: Snapshot, base: BaseState| Waited {
        outcome,
        exit_code: outcome.exit_code(),
        pull_request: pr.url.to_string(),
        new_head: (look.head != head).then(|| look.head.clone()),
        head: head.clone(),
        base,
        checks: look.checks,
        failures: Vec::new(),
        state: look.state,
    };
    if !is_open(first.state) || (options.ready_only && first.state != PrState::Open) {
        let waited = ended(Outcome::NotOpen, first, BaseState::Unknown);
        activity(&activity_text(&waited));
        return Ok(waited);
    }
    activity("Waiting for CI");
    let waited = loop {
        let mut seen = look(pr, clock, deadline, progress)?;
        if !is_open(seen.state) {
            seen.checks.clear();
            break ended(Outcome::NotOpen, seen, BaseState::Unknown);
        }
        if seen.head != head {
            // The checks belong to another commit; they say nothing about this one.
            seen.checks.clear();
            break ended(Outcome::HeadChanged, seen, BaseState::Unknown);
        }
        let pending = seen
            .checks
            .iter()
            .filter(|check| check.bucket == "pending")
            .count();
        let failed = seen.checks.iter().any(|check| check.bucket == "fail");
        let outcome = if !seen.applies {
            if waited_for() >= GRACE.min(options.timeout) {
                Some(Outcome::NoCi)
            } else {
                progress("No checks reported yet");
                None
            }
        } else if pending > 0 && !(options.fail_fast && failed) {
            if waited_for() >= options.timeout {
                Some(Outcome::Timeout)
            } else {
                progress(&format!(
                    "{pending} of {} checks still running",
                    seen.checks.len()
                ));
                None
            }
        } else if failed {
            Some(Outcome::Failed)
        } else if seen.checks.iter().any(|check| check.bucket == "cancel") {
            Some(Outcome::Cancelled)
        } else {
            Some(Outcome::Passed)
        };
        let Some(outcome) = outcome else {
            clock.sleep(POLL.min(options.timeout.saturating_sub(waited_for())));
            continue;
        };
        let base = base_state(pr, clock, deadline, progress, seen.base)?;
        let mut waited = ended(outcome, seen, base);
        if outcome == Outcome::Failed {
            waited.failures = pr.adapter.failures(pr.tools, &waited.checks);
        }
        break waited;
    };
    activity(&activity_text(&waited));
    Ok(waited)
}

/// `base`, or while the host is still working it out, the base state of a new snapshot, asked for a few
/// times before it counts as unknown.
fn base_state(
    pr: &PullRequest,
    clock: &dyn Clock,
    deadline: Duration,
    progress: &mut dyn FnMut(&str),
    mut base: Option<BaseState>,
) -> Result<BaseState, ToolProblem> {
    for _ in 0..BASE_RETRIES {
        if let Some(base) = base {
            return Ok(base);
        }
        clock.sleep(BASE_RETRY);
        base = look(pr, clock, deadline, progress)?.base;
    }
    Ok(base.unwrap_or(BaseState::Unknown))
}

/// One snapshot of `pr`. When the host fails to answer, it is asked again a few times, ten seconds apart.
/// It never sleeps past `deadline` (a time on the clock); without time left, or after the last try, the
/// last problem is returned.
fn look(
    pr: &PullRequest,
    clock: &dyn Clock,
    deadline: Duration,
    progress: &mut dyn FnMut(&str),
) -> Result<Snapshot, ToolProblem> {
    let mut retries = 0;
    loop {
        match pr.snapshot() {
            Err(problem)
                if retries < SNAPSHOT_RETRIES && clock.elapsed() + SNAPSHOT_RETRY <= deadline =>
            {
                retries += 1;
                progress(&format!(
                    "The host did not answer ({}); asking again in {}s",
                    problem.0,
                    SNAPSHOT_RETRY.as_secs()
                ));
                clock.sleep(SNAPSHOT_RETRY);
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests;
