//! Gates: the conditions a run must meet before it may leave a Stage.
//!
//! The CLI owns them (ADR-0001), and an accepted exit carries their evidence (ADR-0019).
//!
//! The gate interface is what this module re-exports. Behind it each job has its own module:
//!
//! - `request`: the requests of `jidoka run gate` and `jidoka run advance` and their options;
//! - `local`: recording gate results and evaluating the exit gates the record and the worktree decide;
//! - `review_selection`: which Reviews apply to the paths the run changed;
//! - `ci`: the live CI check that lets a run leave Finish.
//!
//! [`accepted_exit`] puts the local and live evaluations together for an advance. Where an accepted exit
//! leads, and what its Stage exit records, is decided by [`Run::leave`] alone.

mod ci;
mod local;
mod request;
mod review_selection;
#[cfg(test)]
mod tests;

pub(crate) use local::{AssessmentPurpose, assess_local, latest_ticket_verification, record};
pub(crate) use request::{AdvanceCommand, AdvanceRequest, GateCommand, Record};
pub(crate) use review_selection::ReviewSelection;

use crate::{
    git::Git,
    pr::host::PrHost,
    run::{Exit, Run},
    stages::Gate,
};
use local::LocalAssessment;

/// The remedy `advance` names for this gate, which fails either on open Findings or on no recorded pass.
const RECORD_OR_RESOLVE_FINDINGS: &str = "record `jidoka run gate <id> no-actionable-findings pass` once no Finding is open; resolve open Findings with `jidoka run update <id> --finding-status <finding-id>=<fixed|dismissed|deferred>[:<reason>]` or start it with `jidoka run rework`";

/// Evaluates the exit gates of the current Stage and returns their evidence as an accepted exit.
///
/// Live CI is included when the gates name it. Where the exit leads is for [`Run::leave`] to decide.
///
/// # Errors
///
/// Returns a message naming each failing gate and its remedy, or why CI has not passed.
fn accepted_exit(run: &Run, git: &dyn Git, host: &dyn PrHost) -> Result<Exit, String> {
    exit_with(run, git, CiCheck::Live(host))
}

/// How an exit from Finish settles the CI gate.
#[derive(Clone, Copy)]
enum CiCheck<'a> {
    /// Check the pull request's CI live.
    Live(&'a dyn PrHost),
    /// Skip only the CI gate, for the given reason; every other gate still applies.
    Skip(&'a str),
}

/// [`accepted_exit`] with the CI gate either checked live or skipped.
fn exit_with(run: &Run, git: &dyn Git, ci_check: CiCheck) -> Result<Exit, String> {
    let LocalAssessment {
        transition,
        evaluations: gates,
        reviews,
        candidate,
    } = assess_local(run, git, AssessmentPurpose::Advance);
    let transition = transition?;
    let failures: Vec<_> = gates
        .iter()
        .filter_map(|gate| {
            gate.reason
                .as_ref()
                .map(|reason| gate_failure(run, gate.name, reason))
        })
        .collect();
    if !failures.is_empty() {
        return Err(format!("Cannot advance:\n{}", failures.join("\n")));
    }
    if let Some(Ok(before)) = candidate
        && run.worktree_head(git)? != before
    {
        return Err("The worktree HEAD changed while checking gates; try again".into());
    }

    let has_ci_gate = transition.gates.contains(&Gate::CiPassed);
    if let CiCheck::Skip(reason) = ci_check {
        if !has_ci_gate {
            return Err("--skip-ci only applies when leaving Finish".into());
        }
        return Ok(Exit::CiSkipped {
            gates,
            skipped_checks: skipped_snapshot_of(reviews),
            head: run.worktree_head(git)?,
            reason: reason.to_owned(),
        });
    }
    let ci = if let (true, CiCheck::Live(host)) = (has_ci_gate, ci_check) {
        let ci = ci::ci_passed(run, git, host).map_err(|reason| {
            format!(
                "Cannot advance:\n{}",
                gate_failure(run, Gate::CiPassed, &reason)
            )
        })?;
        Some(ci)
    } else {
        None
    };
    Ok(Exit::Accepted {
        gates,
        skipped_checks: skipped_snapshot_of(reviews),
        ci,
    })
}

fn skipped_snapshot_of(reviews: ReviewSelection) -> Vec<crate::run::SkippedReview> {
    match reviews {
        ReviewSelection::Selected { selection, .. } => {
            review_selection::skipped_snapshot(&selection)
        }
        ReviewSelection::NotApplicable | ReviewSelection::Unavailable(_) => Vec::new(),
    }
}

fn gate_failure(run: &Run, gate: Gate, reason: &str) -> String {
    format!(
        "{}: {reason}; {}",
        gate.name(),
        gate.remedy().replace("<id>", &run.id)
    )
}

impl Gate {
    /// What to do so that this gate passes, with `<id>` standing for the run id.
    fn remedy(self) -> &'static str {
        match self {
            Self::TicketsDone => {
                r#"list tickets with `jidoka run update <id> --ticket "<ref> <title>"` and mark them done with `--ticket-status <ref>=done`, or declare `--unticketed`"#
            }
            Self::Verified => {
                "run full verification and record `jidoka run gate <id> verified pass`"
            }
            Self::TicketsVerified => {
                "mark the listed ticket in progress or declare unticketed work, then verify and run `jidoka run gate <id> tickets-verified pass`"
            }
            Self::ChecksRecorded => {
                "record each result with `jidoka run gate <id> checks-recorded pass --check <name>`"
            }
            Self::NoActionableFindings => RECORD_OR_RESOLVE_FINDINGS,
            Self::FinishSummary => "record `jidoka run update <id> --summary finish=<text>`",
            Self::CiPassed => "open the pull request and pass CI; run `jidoka pr wait <id>`",
        }
    }
}

fn require(condition: bool, reason: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(reason.to_string())
    }
}
