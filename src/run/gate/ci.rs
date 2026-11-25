//! CI evaluation: the live check of the pull request's CI that lets a run leave Finish.

use super::require;
use crate::{
    git::Git,
    pr,
    pr::host::PrHost,
    run::{CiEvidence, CiOutcome, Run, updated_now},
};
use std::time::{Duration, Instant};

/// Allow the existing 90-second CI discovery grace period, including a final poll.
const CI_TIMEOUT: Duration = Duration::from_secs(95);

/// Watches the pull request's CI on the worktree HEAD until it has one outcome.
///
/// Returns it as evidence when CI passed or the project has no CI.
pub(super) fn ci_passed(run: &Run, git: &dyn Git, host: &dyn PrHost) -> Result<CiEvidence, String> {
    let url = run.pr.as_deref().ok_or("No pull request is recorded")?;
    let checked_at = updated_now().map_err(|error| format!("Could not read the time: {error}"))?;
    let commit = run.worktree_head(git)?;
    let options = pr::wait::Options {
        timeout: CI_TIMEOUT,
        pushed_head: None,
        ready_only: true,
        ..Default::default()
    };
    let clock = pr::wait::RealClock(Instant::now());
    let result = host
        .watch(url, &options, &clock, &mut |_| {}, &mut |_| {})
        .map_err(|error| format!("Could not check CI: {}", error.0))?;
    require(
        result.state == pr::state::PrState::Open,
        &format!(
            "Pull request is {}; if it merged, run `jidoka run sync {}` to reconcile the run",
            result.state.label(),
            run.id
        ),
    )?;
    require(
        result.head == commit,
        "The pull request head does not match the worktree HEAD",
    )?;
    let outcome = match result.outcome {
        pr::wait::Outcome::Passed => CiOutcome::Passed,
        pr::wait::Outcome::NoCi => CiOutcome::NoCi,
        other => return Err(format!("CI outcome is {other:?}")),
    };
    require(
        run.worktree_head(git)? == commit,
        "The worktree HEAD changed while checking CI; try again",
    )?;
    Ok(CiEvidence {
        head: commit,
        outcome,
        pr_status: pr::state::PrStatus {
            url: url.into(),
            state: result.state,
            reason: None,
            checked_at,
        },
    })
}
