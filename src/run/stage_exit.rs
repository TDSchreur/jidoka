//! Leaving a Stage: the one way a run moves along the Stage graph, and the evidence it records.

use super::{
    CiOutcome, ClassifiedFinding, Completion, FindingClassification, FindingState, MAX_NOTE_BYTES,
    MAX_REPORTS_PER_RUN, MAX_REWORK, MAX_TEXT_BYTES, ReportKind, Run, RunReport, RunStatus, Stage,
    Ticket, check_text,
};
use crate::{
    pr_state::PrStatus,
    stages::{self, Destination, Gate, Transition},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Bound the gate history, which is normally only a handful of entries per stage.
pub(crate) const MAX_GATE_RESULTS: usize = 1000;

/// ADR-0006 requires a decision for behavior changes in the third Rework.
const WORDING_REWORK: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GateResult {
    pub(crate) gate: Gate,
    pub(crate) stage: Stage,
    pub(crate) rework: Option<u32>,
    pub(crate) passed: bool,
    pub(crate) evidence: Option<String>,
    pub(crate) commit: Option<String>,
    pub(crate) ticket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) work_attempt: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) check: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) skipped_checks: Vec<SkippedReview>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SkippedReview {
    pub(crate) name: String,
    pub(crate) reason: String,
}

/// A gate evaluated against the current stage pass or copied into its exit record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Evaluation {
    pub(crate) name: Gate,
    pub(crate) status: GateStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) results: Vec<GateResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GateStatus {
    Open,
    Failed,
    Passed,
    AwaitingLiveCheck,
    Bypassed,
    Unknown,
}

impl Evaluation {
    pub(crate) fn with_status(name: Gate, status: GateStatus) -> Self {
        Self {
            name,
            status,
            reason: None,
            detail: None,
            results: Vec::new(),
        }
    }
}

/// The immutable evidence for one accepted move out of a stage pass.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StageExit {
    pub(crate) stage: Stage,
    pub(crate) rework: Option<u32>,
    pub(crate) to: Option<Stage>,
    pub(crate) at: u64,
    #[serde(flatten)]
    pub(crate) outcome: ExitOutcome,
    pub(crate) gates: Vec<Evaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) build_plan: Option<BuildPlan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) skipped_checks: Vec<SkippedReview>,
}

/// Work recorded when a Build pass ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum BuildPlan {
    Unplanned,
    Unticketed { attempt: u64 },
    Ticketed { tickets: Vec<Ticket> },
}

impl BuildPlan {
    fn from_run(run: &Run) -> Self {
        if let Some(attempt) = run.unticketed_attempt {
            Self::Unticketed { attempt }
        } else if run.tickets.is_empty() {
            Self::Unplanned
        } else {
            Self::Ticketed {
                tickets: run.tickets.clone(),
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub(crate) enum ExitOutcome {
    Accepted,
    Override {
        reason: String,
    },
    Merged {
        url: String,
    },
    Rework {
        number: u32,
        #[serde(flatten)]
        included: ReworkFindings,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ReworkRecord {
    pub(crate) number: u32,
    pub(crate) at: u64,
    #[serde(flatten)]
    pub(crate) included: ReworkFindings,
    pub(crate) classification: FindingClassification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) decision: Option<String>,
}

/// Finding evidence saved when Check starts a Rework.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum ReworkFindings {
    Classified {
        findings: Vec<ClassifiedFinding>,
    },
    Historical {
        finding: String,
        // Records saved before classifications hold the triggering Finding and, in history, the open IDs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        findings: Option<Vec<String>>,
    },
}

/// The live CI result that lets a run leave Finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CiEvidence {
    /// The commit CI checked, which is also the worktree HEAD.
    pub(crate) head: String,
    pub(crate) outcome: CiOutcome,
    /// The pull request state read while checking CI.
    pub(crate) pr_status: PrStatus,
}

/// Why a run leaves its current Stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    /// Every exit gate passed, so the run moves to the next Stage. Leaving Finish also needs the live CI
    /// result, which completes the run; no other Stage carries one.
    Accepted {
        gates: Vec<Evaluation>,
        skipped_checks: Vec<SkippedReview>,
        ci: Option<CiEvidence>,
    },
    /// The user moved the run on without checking the gates.
    Override { reason: String },
    /// The pull request merged, which finishes a run in Finish even while it is blocked.
    Merged { url: String },
    /// Check found something to fix: back to Build.
    Rework {
        findings: Vec<ClassifiedFinding>,
        decision: Option<String>,
    },
}

/// Where an exit leads and what its Stage exit record carries.
struct Leaving {
    to: Destination,
    outcome: ExitOutcome,
    gates: Vec<Evaluation>,
    skipped_checks: Vec<SkippedReview>,
}

impl Run {
    /// Leaves the current Stage at `at`: the only way a run moves to another Stage or finishes.
    ///
    /// Checks the exit's policy, records the Stage exit, and moves the run to the next Stage, or completes
    /// it with `finished_at` set to `at`. Only a running run can leave a Stage, except a merge in Finish.
    ///
    /// # Errors
    ///
    /// Returns a message when the run cannot leave its Stage this way; the run is then unchanged.
    pub(crate) fn leave(&mut self, exit: Exit, at: u64) -> Result<(), String> {
        let stage = self.stage;
        let rework = self.rework;
        let build_plan = (stage == Stage::Build).then(|| BuildPlan::from_run(self));
        // Every exit but a merge follows the mode's Stage graph.
        let mode = self.mode.ok_or("Record the run's mode first");
        let Leaving {
            to,
            outcome,
            gates,
            skipped_checks,
        } = match exit {
            Exit::Accepted {
                mut gates,
                skipped_checks,
                ci,
            } => {
                let to = stages::next(mode?, stage, self.status)?.to;
                match (to, ci) {
                    (Destination::Done, Some(ci)) => self.finish_with_ci(&mut gates, ci)?,
                    (Destination::Done, None) => {
                        return Err("Finish needs a live CI result".into());
                    }
                    (Destination::Stage(_), Some(_)) => {
                        return Err("Only an exit from Finish carries a CI result".into());
                    }
                    (Destination::Stage(_), None) => {}
                }
                Leaving {
                    to,
                    outcome: ExitOutcome::Accepted,
                    gates,
                    skipped_checks,
                }
            }
            Exit::Override { reason } => {
                let reason = reason.trim().to_owned();
                let transition = stages::next(mode?, stage, self.status)?;
                let gates = self.force(&reason, transition, at)?;
                Leaving {
                    to: transition.to,
                    outcome: ExitOutcome::Override { reason },
                    gates,
                    skipped_checks: Vec::new(),
                }
            }
            Exit::Merged { url } => Leaving {
                to: Destination::Done,
                gates: self.finish_from_merge(&url)?,
                outcome: ExitOutcome::Merged { url },
                skipped_checks: Vec::new(),
            },
            Exit::Rework { findings, decision } => {
                let mode = mode.map_err(|_| "Record the run's mode before starting a Rework")?;
                let to = stages::rework(mode, stage, self.status)?.to;
                let record = self.start_rework(findings, decision, at)?;
                let outcome = ExitOutcome::Rework {
                    number: record.number,
                    included: record.included.clone(),
                };
                self.rework_history.push(record);
                Leaving {
                    to,
                    outcome,
                    gates: Vec::new(),
                    skipped_checks: Vec::new(),
                }
            }
        };
        self.stage_exits.push(StageExit {
            stage,
            rework,
            to: match to {
                Destination::Stage(stage) => Some(stage),
                Destination::Done => None,
            },
            at,
            outcome,
            gates,
            build_plan,
            skipped_checks,
        });
        match to {
            Destination::Stage(next) => self.stage = next,
            Destination::Done => self.finished_at = Some(at),
        }
        Ok(())
    }

    /// Completes the run out of Finish with its CI evidence, marking the CI gate passed.
    fn finish_with_ci(&mut self, gates: &mut [Evaluation], ci: CiEvidence) -> Result<(), String> {
        if self.gates.len() >= MAX_GATE_RESULTS {
            return Err(format!(
                "A run can hold at most {MAX_GATE_RESULTS} gate results"
            ));
        }
        let gate = gates
            .iter_mut()
            .find(|gate| gate.name == Gate::CiPassed)
            .ok_or("Finish has no CI gate")?;
        let short: String = ci.head.chars().take(7).collect();
        let result = GateResult {
            gate: Gate::CiPassed,
            stage: self.stage,
            rework: self.rework,
            passed: true,
            evidence: Some(format!("{} on {short}", ci.outcome.label())),
            commit: Some(ci.head.clone()),
            ticket: None,
            work_attempt: None,
            check: None,
            skipped_checks: Vec::new(),
        };
        self.complete(Completion::Ci {
            head: ci.head,
            outcome: ci.outcome,
        })?;
        gate.status = GateStatus::Passed;
        gate.results.push(result.clone());
        self.gates.push(result);
        if self
            .pr_status
            .as_ref()
            .is_none_or(|previous| ci.pr_status.supersedes(previous))
        {
            self.pr_status = Some(ci.pr_status);
        }
        Ok(())
    }

    /// Checks a forced `transition`, records `reason` as a run report and returns its bypassed gates.
    fn force(
        &mut self,
        reason: &str,
        transition: Transition,
        at: u64,
    ) -> Result<Vec<Evaluation>, String> {
        if reason.is_empty() {
            return Err("Provide a reason for the forced advance".into());
        }
        let text = format!("Forced advance from {}: {reason}", self.stage.name());
        if text.len() > MAX_TEXT_BYTES || text.chars().any(char::is_control) {
            return Err(format!(
                "The reason must be short ({MAX_TEXT_BYTES} bytes with the prefix) and contain no control characters"
            ));
        }
        if self.run_reports.len() >= MAX_REPORTS_PER_RUN {
            return Err(format!(
                "A run can hold at most {MAX_REPORTS_PER_RUN} run reports"
            ));
        }
        if transition.to == Destination::Done {
            self.complete(Completion::Override {
                reason: reason.into(),
            })?;
        }
        let gates = transition
            .gates
            .iter()
            .map(|&gate| {
                if gate == Gate::FinishSummary {
                    self.finish_summary()
                } else {
                    Evaluation::with_status(gate, GateStatus::Bypassed)
                }
            })
            .collect();
        self.run_reports.push(RunReport {
            kind: ReportKind::Activity,
            text,
            stage: self.stage,
            at,
        });
        Ok(gates)
    }

    /// Completes a run in Finish whose pull request merged, even while it is blocked.
    fn finish_from_merge(&mut self, url: &str) -> Result<Vec<Evaluation>, String> {
        self.complete(Completion::Merged {
            url: url.to_owned(),
        })?;
        Ok(vec![
            self.finish_summary(),
            Evaluation::with_status(Gate::CiPassed, GateStatus::Bypassed),
        ])
    }

    /// Validates the included Findings and decision before assigning the Rework.
    fn start_rework(
        &mut self,
        findings: Vec<ClassifiedFinding>,
        decision: Option<String>,
        at: u64,
    ) -> Result<ReworkRecord, String> {
        let number = next_rework(self.rework)?;
        self.check_rework_findings(&findings)?;
        let classification = if findings
            .iter()
            .any(|finding| finding.classification == FindingClassification::Behavior)
        {
            FindingClassification::Behavior
        } else {
            FindingClassification::Wording
        };
        if let Some(decision) = &decision {
            check_text("Rework decision", decision, MAX_NOTE_BYTES)?;
        }
        if rework_requires_decision(number, classification) && decision.is_none() {
            return Err(format!(
                "Rework {number} requires the user's decision; record it with --decision"
            ));
        }
        for open in self
            .findings
            .iter_mut()
            .filter(|finding| finding.state == FindingState::Open)
        {
            open.rework = Some(number);
        }
        self.rework = Some(number);
        Ok(ReworkRecord {
            number,
            at,
            included: ReworkFindings::Classified { findings },
            classification,
            decision,
        })
    }

    /// Checks that the request classifies exactly the current open Findings.
    fn check_rework_findings(&self, findings: &[ClassifiedFinding]) -> Result<(), String> {
        let retry = "Inspect the run and submit one classification for each open Finding";
        if findings.is_empty() {
            return Err(format!(
                "Rework requires at least one classified open Finding. {retry}"
            ));
        }
        let mut open: BTreeSet<_> = self
            .open_findings()
            .map(|finding| finding.id.as_str())
            .collect();
        let mut seen = BTreeSet::new();
        for finding in findings {
            if !seen.insert(finding.id.as_str()) {
                return Err(format!(
                    "Finding {} was classified more than once. {retry}",
                    finding.id
                ));
            }
            if !open.remove(finding.id.as_str()) {
                return Err(format!(
                    "Finding {} is not open on this run. {retry}",
                    finding.id
                ));
            }
        }
        if !open.is_empty() {
            let missing = open.into_iter().collect::<Vec<_>>().join(", ");
            return Err(format!(
                "Missing classifications for open Findings: {missing}. {retry}"
            ));
        }
        Ok(())
    }

    /// Completes Finish with evidence, retaining the summary or supplying one for a merge.
    fn complete(&mut self, completion: Completion) -> Result<(), String> {
        if self.stage != Stage::Finish || self.status == RunStatus::Done {
            return Err("Only an unfinished run in Finish can be completed".into());
        }
        if matches!(completion, Completion::Merged { .. }) && self.check_finish_summary().is_err() {
            self.summaries
                .insert(Stage::Finish, "Pull request merged".into());
        }
        self.check_finish_summary()?;
        self.status = RunStatus::Done;
        self.completion = Some(completion);
        self.activity = None;
        self.blocking_reason = None;
        self.block_source = None;
        Ok(())
    }

    fn finish_summary(&self) -> Evaluation {
        let mut summary = Evaluation::with_status(Gate::FinishSummary, GateStatus::Passed);
        summary.detail = self.summaries.get(&Stage::Finish).cloned();
        summary
    }
}

/// Applies the decision threshold from ADR-0006 to the complete Rework.
fn rework_requires_decision(number: u32, classification: FindingClassification) -> bool {
    number > WORDING_REWORK
        || (number == WORDING_REWORK && classification == FindingClassification::Behavior)
}

/// The number of the next Rework, refused past the Rework limit.
///
/// # Errors
///
/// Returns a message when the run has already reached the Rework limit.
pub(crate) fn next_rework(current: Option<u32>) -> Result<u32, String> {
    let count = current.unwrap_or_default();
    if count >= MAX_REWORK {
        return Err(format!(
            "The run has reached the limit of {MAX_REWORK} Reworks"
        ));
    }
    Ok(count + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pr_state::PrState,
        run::{
            Finding, Mode,
            tests::{finding, sample_run},
        },
    };

    const AT: u64 = 1_700_000_000_000;

    fn run_in(stage: Stage) -> Run {
        let mut run = sample_run();
        run.mode = Some(Mode::Balanced);
        run.stage = stage;
        run.summaries
            .insert(Stage::Finish, "Implemented the task".into());
        run
    }

    fn fixed(id: &str) -> Finding {
        Finding {
            state: FindingState::Fixed,
            ..finding(id)
        }
    }

    fn json(run: &Run) -> serde_json::Value {
        serde_json::to_value(run).unwrap()
    }

    #[test]
    fn an_accepted_exit_from_finish_completes_the_run_with_its_ci_result() {
        let mut run = run_in(Stage::Finish);
        let status = PrStatus {
            url: "https://example.test/pr/1".into(),
            state: PrState::Open,
            reason: None,
            checked_at: 5,
        };
        run.leave(
            Exit::Accepted {
                gates: vec![
                    Evaluation::with_status(Gate::FinishSummary, GateStatus::Passed),
                    Evaluation::with_status(Gate::CiPassed, GateStatus::AwaitingLiveCheck),
                ],
                skipped_checks: Vec::new(),
                ci: Some(CiEvidence {
                    head: "abc1234def".into(),
                    outcome: CiOutcome::Passed,
                    pr_status: status.clone(),
                }),
            },
            AT,
        )
        .unwrap();
        assert_eq!((run.stage, run.status), (Stage::Finish, RunStatus::Done));
        assert_eq!(run.finished_at, Some(AT));
        assert_eq!(
            run.completion,
            Some(Completion::Ci {
                head: "abc1234def".into(),
                outcome: CiOutcome::Passed
            })
        );
        assert_eq!(run.pr_status, Some(status));
        let exit = &run.stage_exits[0];
        assert_eq!((exit.stage, exit.to, exit.at), (Stage::Finish, None, AT));
        assert_eq!(exit.outcome, ExitOutcome::Accepted);
        assert_eq!(exit.gates[1].status, GateStatus::Passed);
        assert_eq!(exit.gates[1].results, run.gates);
        assert_eq!(
            run.gates[0].evidence.as_deref(),
            Some("CI passed on abc1234")
        );
    }

    #[test]
    fn an_accepted_exit_from_finish_keeps_a_merged_state_recorded_meanwhile() {
        let mut run = run_in(Stage::Finish);
        let observed = PrStatus {
            url: "https://example.test/pr/1".into(),
            state: PrState::Open,
            reason: None,
            checked_at: 5,
        };
        let merged = PrStatus {
            state: PrState::Merged,
            checked_at: 4,
            ..observed.clone()
        };
        run.pr_status = Some(merged.clone());
        run.leave(
            Exit::Accepted {
                gates: vec![Evaluation::with_status(
                    Gate::CiPassed,
                    GateStatus::AwaitingLiveCheck,
                )],
                skipped_checks: Vec::new(),
                ci: Some(CiEvidence {
                    head: "abc1234".into(),
                    outcome: CiOutcome::Passed,
                    pr_status: observed,
                }),
            },
            AT,
        )
        .unwrap();
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(run.pr_status, Some(merged));
    }

    #[test]
    fn an_override_moves_on_with_bypassed_gates_and_a_run_report() {
        let mut run = run_in(Stage::Check);
        run.leave(
            Exit::Override {
                reason: " Review unavailable ".into(),
            },
            AT,
        )
        .unwrap();
        assert_eq!(run.stage, Stage::Finish);
        assert_eq!(run.finished_at, None);
        let exit = &run.stage_exits[0];
        assert_eq!((exit.stage, exit.to), (Stage::Check, Some(Stage::Finish)));
        assert_eq!(
            exit.outcome,
            ExitOutcome::Override {
                reason: "Review unavailable".into()
            }
        );
        assert!(
            exit.gates
                .iter()
                .all(|gate| gate.status == GateStatus::Bypassed)
        );
        let report = run.run_reports.last().unwrap();
        assert_eq!(report.text, "Forced advance from check: Review unavailable");
        assert_eq!((report.stage, report.at), (Stage::Check, AT));

        run.leave(
            Exit::Override {
                reason: "CI unavailable".into(),
            },
            AT + 1,
        )
        .unwrap();
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(run.finished_at, Some(AT + 1));
        assert_eq!(
            run.completion,
            Some(Completion::Override {
                reason: "CI unavailable".into()
            })
        );
        assert_eq!(run.stage_exits[1].to, None);
    }

    #[test]
    fn a_merge_finishes_a_blocked_run_in_finish() {
        let mut run = run_in(Stage::Finish);
        run.summaries.clear();
        run.block(
            "PR closed without merge",
            crate::run::LifecycleSource::PullRequest,
            1,
        )
        .unwrap();
        let url = "https://example.test/pr/1".to_string();
        run.leave(Exit::Merged { url: url.clone() }, AT).unwrap();
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(run.finished_at, Some(AT));
        assert_eq!(run.blocking_reason, None);
        assert_eq!(
            run.completion,
            Some(Completion::Merged { url: url.clone() })
        );
        let exit = &run.stage_exits[0];
        assert_eq!((exit.stage, exit.to, exit.at), (Stage::Finish, None, AT));
        assert_eq!(exit.outcome, ExitOutcome::Merged { url });
        assert_eq!(exit.gates[0].detail.as_deref(), Some("Pull request merged"));
        assert_eq!(exit.gates[1].status, GateStatus::Bypassed);
    }

    fn classified(items: &[(&str, FindingClassification)]) -> Vec<ClassifiedFinding> {
        items
            .iter()
            .map(|(id, classification)| ClassifiedFinding {
                id: (*id).into(),
                classification: *classification,
            })
            .collect()
    }

    fn exit_included(exit: &StageExit) -> ReworkFindings {
        match &exit.outcome {
            ExitOutcome::Rework { included, .. } => included.clone(),
            other => panic!("not a Rework exit: {other:?}"),
        }
    }

    #[test]
    fn historical_rework_records_still_load() {
        let old = r#"{"number":1,"at":5,"finding":"F1","findings":["F1","F2"],"classification":"wording"}"#;
        let record: ReworkRecord = serde_json::from_str(old).unwrap();
        assert!(matches!(record.included, ReworkFindings::Historical { .. }));
        let old = r#"{"outcome":"rework","number":1,"finding":"F1"}"#;
        let outcome: ExitOutcome = serde_json::from_str(old).unwrap();
        assert!(matches!(
            outcome,
            ExitOutcome::Rework {
                included: ReworkFindings::Historical { .. },
                ..
            }
        ));
    }

    #[test]
    fn a_rework_returns_to_build_and_records_its_history() {
        let mut run = run_in(Stage::Check);
        run.findings = vec![finding("F1"), fixed("F2"), finding("F3")];
        run.leave(
            Exit::Rework {
                findings: classified(&[
                    ("F1", FindingClassification::Wording),
                    ("F3", FindingClassification::Wording),
                ]),
                decision: None,
            },
            AT,
        )
        .unwrap();
        assert_eq!((run.stage, run.rework), (Stage::Build, Some(1)));
        assert_eq!(run.finished_at, None);
        let exit = &run.stage_exits[0];
        assert_eq!((exit.stage, exit.rework), (Stage::Check, None));
        assert_eq!((exit.to, exit.at), (Some(Stage::Build), AT));
        assert_eq!(
            exit.outcome,
            ExitOutcome::Rework {
                number: 1,
                included: ReworkFindings::Classified {
                    findings: classified(&[
                        ("F1", FindingClassification::Wording),
                        ("F3", FindingClassification::Wording)
                    ])
                }
            }
        );
        let tagged: Vec<_> = run.findings.iter().map(|f| f.rework).collect();
        assert_eq!(tagged, [Some(1), None, Some(1)]);
        let history = &run.rework_history[0];
        assert_eq!((history.number, history.at), (1, AT));
        assert_eq!(history.included, exit_included(exit));
    }

    #[test]
    fn a_rework_without_a_mode_asks_for_the_mode() {
        let mut run = run_in(Stage::Check);
        run.mode = None;
        let result = run.leave(
            Exit::Rework {
                findings: classified(&[("F1", FindingClassification::Wording)]),
                decision: None,
            },
            AT,
        );
        assert_eq!(
            result,
            Err("Record the run's mode before starting a Rework".into())
        );
    }

    #[test]
    fn a_refused_exit_leaves_the_run_unchanged() {
        let rework = |finding: &str, decision: Option<&str>| Exit::Rework {
            findings: classified(&[(finding, FindingClassification::Behavior)]),
            decision: decision.map(str::to_owned),
        };
        let mut check = run_in(Stage::Check);
        check.findings = vec![finding("F1"), fixed("F2")];
        check.rework = Some(WORDING_REWORK);
        let mut blocked = check.clone();
        blocked
            .block("waiting", crate::run::LifecycleSource::Agent, 1)
            .unwrap();
        let accepted = |ci: Option<CiEvidence>| Exit::Accepted {
            gates: vec![Evaluation::with_status(
                Gate::CiPassed,
                GateStatus::AwaitingLiveCheck,
            )],
            skipped_checks: Vec::new(),
            ci,
        };
        let ci = CiEvidence {
            head: "abc1234".into(),
            outcome: CiOutcome::Passed,
            pr_status: PrStatus {
                url: "https://example.test/pr/1".into(),
                state: PrState::Open,
                reason: None,
                checked_at: 5,
            },
        };
        let cases = [
            (
                check.clone(),
                accepted(Some(ci)),
                "Only an exit from Finish carries a CI result",
            ),
            (
                run_in(Stage::Finish),
                accepted(None),
                "Finish needs a live CI result",
            ),
            (check.clone(), rework("F2", Some("go")), "F2 is not open"),
            (
                check.clone(),
                rework("F1", None),
                "requires the user's decision",
            ),
            (
                check.clone(),
                Exit::Override {
                    reason: "x".repeat(MAX_TEXT_BYTES),
                },
                "must be short",
            ),
            (blocked.clone(), rework("F1", Some("go")), "is blocked"),
            (
                blocked.clone(),
                Exit::Override {
                    reason: "Stuck".into(),
                },
                "is blocked",
            ),
            (
                blocked,
                Exit::Merged {
                    url: "https://example.test/pr/1".into(),
                },
                "in Finish",
            ),
        ];
        for (mut run, exit, problem) in cases {
            let before = json(&run);
            let error = run.leave(exit, AT).unwrap_err();
            assert!(error.contains(problem), "{error}");
            assert_eq!(json(&run), before);
        }
    }
}
