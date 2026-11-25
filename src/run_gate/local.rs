//! Local gate evaluation: gates that the run record and the worktree decide.
//!
//! It records gate results and evaluates exit gates, ticket verification included. CI is evaluated live
//! elsewhere.

use super::{
    request::Record,
    require,
    review_selection::{self, ReviewSelection},
};
use crate::{
    git::Git,
    reviews::Selection,
    run::{
        Evaluation, GateResult, GateStatus, MAX_GATE_RESULTS, Mode, Run, RunStatus, Stage, Ticket,
        TicketStatus,
    },
    stages::{self, Gate, Transition},
};
use std::path::Path;

const RESOLVE_FINDINGS: &str = "resolve them with `jidoka run update <id> --finding-status <finding-id>=<fixed|dismissed|deferred>[:<reason>]` or start it with `jidoka run rework`";

/// Refuses while a Finding is open, naming the ids and how to resolve them.
fn no_open_findings(run: &Run) -> Result<(), String> {
    let open: Vec<&str> = run
        .open_findings()
        .map(|finding| finding.id.as_str())
        .collect();
    if open.is_empty() {
        return Ok(());
    }
    Err(format!("Findings {} are open", open.join(", ")))
}

/// Checks `record` against the run and returns the gate result to store.
///
/// # Errors
///
/// Returns a message when the run is not running, the gate is checked automatically, or the evidence,
/// check or worktree does not support the result.
pub(crate) fn record(run: &Run, record: Record, git: &dyn Git) -> Result<GateResult, String> {
    if run.status != RunStatus::Running {
        return Err("Resume the run before recording a gate".into());
    }
    if record
        .evidence
        .as_ref()
        .is_some_and(|s| s.trim().is_empty() || s.len() > 2000)
    {
        return Err("Evidence must be non-empty and at most 2000 bytes".into());
    }
    if record.gate == Gate::NoActionableFindings && record.passed {
        no_open_findings(run).map_err(|problem| format!("{problem}; {RESOLVE_FINDINGS}"))?;
    }
    if matches!(
        record.gate,
        Gate::TicketsDone | Gate::FinishSummary | Gate::CiPassed
    ) {
        return Err("This gate is checked automatically".into());
    }
    if run.gates.len() >= MAX_GATE_RESULTS {
        return Err(format!(
            "A run can hold at most {MAX_GATE_RESULTS} gate results"
        ));
    }
    let check_head_before = if record.gate == Gate::ChecksRecorded {
        Some(run.worktree_head(git)?)
    } else {
        None
    };
    let skipped_checks = if record.gate == Gate::ChecksRecorded {
        if run.stage != Stage::Check {
            return Err("Record check results in the Check stage".into());
        }
        let mode = run.mode.ok_or("Record the run's mode first")?;
        let selection = review_selection::select(run, mode, git)?;
        let name = record
            .check
            .as_deref()
            .ok_or("Use --check <name> to identify the completed check")?;
        if !selection
            .applicable
            .iter()
            .any(|entry| entry.review.name == name && entry.review.gate.gate() == record.gate)
        {
            return Err(format!(
                "Check {name:?} is not applicable to this gate and diff"
            ));
        }
        review_selection::skipped_snapshot(&selection)
    } else {
        if record.check.is_some() {
            return Err("--check is only valid for checks-recorded".into());
        }
        Vec::new()
    };
    let (ticket, work_attempt) = if record.gate == Gate::TicketsVerified {
        let (ticket, attempt) = ticket_verification_target(run)?;
        (ticket.map(|t| t.reference.clone()), Some(attempt))
    } else {
        (None, None)
    };
    let commit = if matches!(
        record.gate,
        Gate::Verified | Gate::ChecksRecorded | Gate::TicketsVerified
    ) && record.passed
    {
        Some(run.worktree_head(git)?)
    } else {
        None
    };
    if let Some(before) = check_head_before
        && run.worktree_head(git)? != before
    {
        return Err("The worktree HEAD changed while selecting checks; try again".into());
    }
    Ok(GateResult {
        gate: record.gate,
        stage: run.stage,
        rework: run.rework,
        passed: record.passed,
        evidence: record.evidence,
        commit,
        ticket,
        work_attempt,
        check: record.check,
        skipped_checks,
    })
}

fn ticket_verification_target(run: &Run) -> Result<(Option<&Ticket>, u64), String> {
    if run.stage != Stage::Build || run.mode != Some(Mode::Full) {
        return Err("Record tickets-verified in Full Build".into());
    }
    if let Some(ticket) = run.active_ticket() {
        return Ok((Some(ticket), ticket.work_attempt));
    }
    if let Some(attempt) = run.unticketed_attempt {
        return Ok((None, attempt));
    }
    Err("Mark a listed ticket in progress before recording its verification, or declare unticketed work".into())
}

fn ticket_plan_recorded(run: &Run) -> Result<(), String> {
    require(
        !run.tickets.is_empty() || run.unticketed_attempt.is_some(),
        "Build has no ticket plan",
    )
}

fn tickets_done(run: &Run) -> Result<(), String> {
    ticket_plan_recorded(run)?;
    require(
        run.tickets
            .iter()
            .all(|ticket| ticket.status == TicketStatus::Done),
        "Some tickets are not done",
    )
}

pub(crate) fn latest_ticket_verification<'a>(
    run: &'a Run,
    ticket: &Ticket,
) -> Option<&'a GateResult> {
    run.gates.iter().rev().find(|result| {
        result.gate == Gate::TicketsVerified
            && result.stage == Stage::Build
            && result.ticket.as_deref() == Some(&ticket.reference)
            && result.work_attempt == Some(ticket.work_attempt)
    })
}

fn unticketed_verification(run: &Run, attempt: u64) -> Option<&GateResult> {
    run.gates.iter().rev().find(|result| {
        result.gate == Gate::TicketsVerified
            && result.stage == Stage::Build
            && result.rework == run.rework
            && result.ticket.is_none()
            && result.work_attempt == Some(attempt)
    })
}

fn tickets_verified<'a>(
    run: &'a Run,
    git: &dyn Git,
    head: &str,
) -> Result<Vec<&'a GateResult>, String> {
    ticket_plan_recorded(run)?;
    if let Some(attempt) = run.unticketed_attempt {
        return unticketed_verification(run, attempt)
            .filter(|result| result.passed && result.commit.as_deref() == Some(head))
            .map(|result| vec![result])
            .ok_or_else(|| {
                "Unticketed Build has no passing verification on its current HEAD".into()
            });
    }
    let worktree = run
        .worktree
        .as_deref()
        .ok_or("Record the run's worktree first")?;
    let mut passed = Vec::new();
    for ticket in &run.tickets {
        let (result, commit) = latest_ticket_verification(run, ticket)
            .filter(|result| result.passed)
            .and_then(|result| Some((result, result.commit.as_deref()?)))
            .ok_or_else(|| {
                format!(
                    "Ticket {} has no passing verification for its current attempt",
                    ticket.reference
                )
            })?;
        if commit != head {
            match git.status(
                Path::new(worktree),
                &["merge-base", "--is-ancestor", commit, head],
            ) {
                Ok(0) => {}
                Ok(1) => {
                    return Err(format!(
                        "Ticket {} was verified on {commit}, which is outside the current Git history",
                        ticket.reference
                    ));
                }
                Ok(code) => {
                    return Err(format!(
                        "Could not check the verified commit {commit} for ticket {}: git exited with {code}",
                        ticket.reference
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "Could not check the verified commit {commit} for ticket {}: {error}",
                        ticket.reference
                    ));
                }
            }
        }
        passed.push(result);
    }
    Ok(passed)
}

fn latest(run: &Run, gate: Gate) -> Option<&GateResult> {
    run.gates
        .iter()
        .rev()
        .find(|r| r.gate == gate && r.stage == run.stage && r.rework == run.rework)
}

fn latest_review<'a>(run: &'a Run, gate: Gate, check: &str) -> Option<&'a GateResult> {
    run.gates.iter().rev().find(|r| {
        r.gate == gate
            && r.stage == run.stage
            && r.rework == run.rework
            && r.check.as_deref() == Some(check)
    })
}

fn review_results<'a>(
    run: &'a Run,
    selection: &Selection,
    gate: Gate,
    head: &str,
) -> Result<Vec<&'a GateResult>, String> {
    selection
        .applicable
        .iter()
        .filter(|entry| entry.review.gate.gate() == gate)
        .map(|entry| {
            latest_review(run, gate, entry.review.name)
                .filter(|result| result.passed && result.commit.as_deref() == Some(head))
                .ok_or_else(|| format!("Check {} has no passing result on HEAD", entry.review.name))
        })
        .collect()
}

/// Determines when to assess a blocked run and whether to preview checks outside Check.
#[derive(Clone, Copy)]
pub(crate) enum AssessmentPurpose {
    Advance,
    Guidance,
    Dashboard,
}

pub(crate) struct LocalAssessment {
    pub(crate) transition: Result<Transition, String>,
    pub(crate) evaluations: Vec<Evaluation>,
    pub(crate) reviews: ReviewSelection,
    /// The worktree HEAD the evaluation used, when a gate needs one.
    pub(super) candidate: Option<Result<String, String>>,
}

/// Collects and evaluates local Stage exit evidence. CI requires a separate live check.
pub(crate) fn assess_local(
    run: &Run,
    git: &dyn Git,
    purpose: AssessmentPurpose,
) -> LocalAssessment {
    let status = match (purpose, run.status) {
        (AssessmentPurpose::Dashboard, RunStatus::Blocked) => RunStatus::Running,
        _ => run.status,
    };
    let transition = run
        .mode
        .ok_or_else(|| "Record the run's mode first".to_string())
        .and_then(|mode| stages::next(mode, run.stage, status));
    let gates = transition.as_ref().map_or(&[][..], |next| next.gates);
    let candidate = candidate(run, gates, git);
    let reviews = review_selection::for_assessment(run, git, purpose);
    let evaluations = evaluate(run, gates, git, &candidate, &reviews);
    LocalAssessment {
        transition,
        evaluations,
        reviews,
        candidate,
    }
}

fn evaluate(
    run: &Run,
    gates: &[Gate],
    git: &dyn Git,
    candidate: &Option<Result<String, String>>,
    selection: &ReviewSelection,
) -> Vec<Evaluation> {
    let candidate = candidate
        .as_ref()
        .ok_or("This stage has no local candidate")
        .and_then(|head| head.as_deref().map_err(String::as_str));
    gates
        .iter()
        .map(|&gate| evaluate_gate(run, gate, git, candidate, selection))
        .collect()
}

fn candidate(run: &Run, gates: &[Gate], git: &dyn Git) -> Option<Result<String, String>> {
    gates
        .iter()
        .any(|gate| {
            matches!(
                gate,
                Gate::Verified | Gate::TicketsVerified | Gate::ChecksRecorded
            )
        })
        .then(|| run.worktree_head(git))
}

/// Relevant recorded results for each subject in a failed or incomplete gate.
fn gate_records<'a>(run: &'a Run, gate: Gate, selection: &ReviewSelection) -> Vec<&'a GateResult> {
    match gate {
        Gate::ChecksRecorded => match selection {
            ReviewSelection::Selected { selection, .. } => selection
                .applicable
                .iter()
                .filter(|entry| entry.review.gate.gate() == gate)
                .filter_map(|entry| latest_review(run, gate, entry.review.name))
                .collect(),
            ReviewSelection::NotApplicable | ReviewSelection::Unavailable(_) => Vec::new(),
        },
        Gate::TicketsVerified => match run.unticketed_attempt {
            Some(attempt) => unticketed_verification(run, attempt).into_iter().collect(),
            None => run
                .tickets
                .iter()
                .filter_map(|ticket| latest_ticket_verification(run, ticket))
                .collect(),
        },
        Gate::Verified | Gate::NoActionableFindings => latest(run, gate).into_iter().collect(),
        Gate::TicketsDone | Gate::FinishSummary | Gate::CiPassed => Vec::new(),
    }
}

fn evaluate_gate(
    run: &Run,
    gate: Gate,
    git: &dyn Git,
    candidate: Result<&str, &str>,
    selection: &ReviewSelection,
) -> Evaluation {
    let mut evaluation = Evaluation::with_status(gate, GateStatus::Passed);
    let result = match gate {
        Gate::TicketsDone => tickets_done(run).map(|()| {
            evaluation.detail = run
                .unticketed_attempt
                .map(|attempt| format!("Unticketed work (attempt {attempt})"))
                .or_else(|| {
                    Some(
                        run.tickets
                            .iter()
                            .map(|ticket| {
                                format!("{} (attempt {})", ticket.reference, ticket.work_attempt)
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                });
            Vec::new()
        }),
        Gate::Verified => passing_record(latest(run, gate)).and_then(|result| {
            require(
                result.commit.as_deref() == Some(candidate?),
                "The verification commit does not match HEAD",
            )?;
            Ok(vec![result])
        }),
        Gate::TicketsVerified => candidate
            .map_err(str::to_owned)
            .and_then(|head| tickets_verified(run, git, head)),
        Gate::ChecksRecorded => match selection {
            ReviewSelection::Selected { selection, .. } => candidate
                .map_err(str::to_owned)
                .and_then(|head| review_results(run, selection, gate, head)),
            ReviewSelection::Unavailable(reason) => Err(reason.clone()),
            ReviewSelection::NotApplicable => Err("This stage has no check selection".into()),
        },
        Gate::NoActionableFindings => no_open_findings(run)
            .and_then(|()| passing_record(latest(run, gate)).map(|result| vec![result])),
        Gate::FinishSummary => run.check_finish_summary().map(|()| {
            evaluation.detail = run.summaries.get(&Stage::Finish).cloned();
            Vec::new()
        }),
        Gate::CiPassed => {
            return Evaluation::with_status(gate, GateStatus::AwaitingLiveCheck);
        }
    };
    match result {
        Ok(results) => evaluation.results = results.into_iter().cloned().collect(),
        Err(reason) => {
            evaluation.reason = Some(reason);
            let failed: Vec<_> = gate_records(run, gate, selection)
                .into_iter()
                .filter(|result| !result.passed)
                .cloned()
                .collect();
            evaluation.status = if failed.is_empty() {
                GateStatus::Open
            } else {
                GateStatus::Failed
            };
            evaluation.results = failed;
        }
    }
    evaluation
}

fn passing_record(recorded: Option<&GateResult>) -> Result<&GateResult, String> {
    recorded
        .filter(|record| record.passed)
        .ok_or_else(|| "No passing result was recorded".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_gate::tests::run;

    #[test]
    fn review_results_are_independent_and_bound_to_the_reviewed_head() {
        static A: crate::reviews::Review = crate::reviews::Review {
            name: "a",
            triggers: &[],
            instructions: "a",
            gate: crate::reviews::ReviewGate::Recorded,
            model_route: None,
            floor: &[],
        };
        static B: crate::reviews::Review = crate::reviews::Review {
            name: "b",
            triggers: &[],
            instructions: "b",
            gate: crate::reviews::ReviewGate::Recorded,
            model_route: None,
            floor: &[],
        };
        let selection = crate::reviews::Selection {
            applicable: vec![
                crate::reviews::Applicable {
                    review: &A,
                    reason: crate::reviews::Reason::Floor,
                },
                crate::reviews::Applicable {
                    review: &B,
                    reason: crate::reviews::Reason::Floor,
                },
            ],
            skipped: vec![],
        };
        let mut run = run(Stage::Check);
        let result = |name: &str, passed| GateResult {
            gate: Gate::ChecksRecorded,
            stage: Stage::Check,
            rework: None,
            passed,
            evidence: None,
            commit: Some("head-1".into()),
            ticket: None,
            work_attempt: None,
            check: Some(name.into()),
            skipped_checks: vec![],
        };
        let check = |run: &Run, head: &str| {
            review_results(run, &selection, Gate::ChecksRecorded, head).map(|results| results.len())
        };
        run.gates.push(result("a", true));
        assert!(check(&run, "head-1").is_err());
        run.gates.push(result("b", true));
        assert_eq!(check(&run, "head-1").unwrap(), 2);
        assert!(check(&run, "head-2").is_err());
        run.gates.push(result("b", false));
        assert!(check(&run, "head-1").is_err());
        run.rework = Some(1);
        assert!(check(&run, "head-1").is_err());
    }
}
