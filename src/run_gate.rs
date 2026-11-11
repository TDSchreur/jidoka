//! Gate evidence and stage transitions.
use crate::{
    pr_state,
    pr_sync::Git,
    pr_wait,
    run::{
        MAX_REPORTS_PER_RUN, MAX_TEXT_BYTES, ReportKind, Run, RunReport, RunStatus, Stage,
        TicketStatus, updated_now,
    },
    stages::{self, Destination, Gate},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// Allow the existing 90-second CI discovery grace period, including a final poll.
const CI_TIMEOUT: Duration = Duration::from_secs(95);

/// Bound the gate history, which is normally only a handful of entries per stage.
const MAX_GATE_RESULTS: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GateResult {
    pub(crate) gate: Gate,
    pub(crate) stage: Stage,
    pub(crate) rework: Option<u32>,
    pub(crate) passed: bool,
    pub(crate) evidence: Option<String>,
    pub(crate) commit: Option<String>,
    pub(crate) ticket: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Record {
    pub(crate) gate: Gate,
    pub(crate) passed: bool,
    pub(crate) evidence: Option<String>,
}

pub(crate) fn head(run: &Run, git: &dyn Git) -> Result<String, String> {
    let path = run
        .worktree
        .as_ref()
        .ok_or("Record the run's worktree first")?;
    git.git(Path::new(path), &["rev-parse", "HEAD"])
        .map(|s| s.trim().to_owned())
        .map_err(|e| format!("Could not read the worktree HEAD: {e}"))
}

pub(crate) fn record(run: &mut Run, record: Record, git: &dyn Git) -> Result<(), String> {
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
    if matches!(record.gate, Gate::TicketsDone | Gate::CiPassed) {
        return Err("This gate is checked automatically".into());
    }
    if run.gates.len() >= MAX_GATE_RESULTS {
        return Err(format!(
            "A run can hold at most {MAX_GATE_RESULTS} gate results"
        ));
    }
    let commit = if record.gate == Gate::Verified && record.passed {
        Some(head(run, git)?)
    } else {
        None
    };
    let ticket = if record.gate == Gate::TicketsVerified {
        Some(
            run.current_ticket
                .clone()
                .ok_or("Record the current ticket before its verification")?,
        )
    } else {
        None
    };
    run.gates.push(GateResult {
        gate: record.gate,
        stage: run.stage,
        rework: run.rework,
        passed: record.passed,
        evidence: record.evidence,
        commit,
        ticket,
    });
    Ok(())
}

pub(crate) fn latest(run: &Run, gate: Gate) -> Option<&GateResult> {
    run.gates
        .iter()
        .rev()
        .find(|r| r.gate == gate && r.stage == run.stage && r.rework == run.rework)
}

pub(crate) fn advance(run: &mut Run, git: &dyn Git, tool: &dyn pr_wait::Gh) -> Result<(), String> {
    advance_with_clock(run, git, tool, &pr_wait::RealClock(Instant::now()))
}

fn advance_with_clock(
    run: &mut Run,
    git: &dyn Git,
    tool: &dyn pr_wait::Gh,
    clock: &dyn pr_wait::Clock,
) -> Result<(), String> {
    let mode = run.mode.ok_or("Record the run's mode first")?;
    let transition = stages::next(mode, run.stage, run.status)?;
    let mut failures = Vec::new();
    for gate in transition.gates {
        let (result, remedy) = match gate {
            Gate::TicketsDone => (
                require(
                    run.tickets.iter().all(|t| t.status == TicketStatus::Done),
                    "Some tickets are not done",
                ),
                "mark every ticket done with `jidoka run update <id> running --ticket-status <ref>=done`",
            ),
            Gate::Verified => (
                verified(run, git),
                "run full verification and record `jidoka run gate <id> verified pass`",
            ),
            Gate::TicketsVerified => (
                require(
                    run.tickets.iter().all(|t| {
                        run.gates
                            .iter()
                            .rev()
                            .find(|r| {
                                r.gate == *gate
                                    && r.stage == run.stage
                                    && r.ticket.as_deref() == Some(&t.reference)
                            })
                            .is_some_and(|r| r.passed)
                    }),
                    "A ticket has no passing verification",
                ),
                "set the current ticket with `jidoka run update <id> running --current-ticket <reference>`, verify it, then run `jidoka run gate <id> tickets-verified pass`",
            ),
            Gate::ChecksRecorded => (
                require(
                    latest(run, *gate).is_some_and(|r| r.passed),
                    "No passing result was recorded",
                ),
                "record `jidoka run gate <id> checks-recorded pass`",
            ),
            Gate::NoActionableFindings => (
                require(
                    latest(run, *gate).is_some_and(|r| r.passed),
                    "No passing result was recorded",
                ),
                "record `jidoka run gate <id> no-actionable-findings pass` or start Rework",
            ),
            Gate::CiPassed => (
                ci_passed(run, git, tool, clock),
                "open the pull request and pass CI; run `jidoka pr wait <id>`",
            ),
        };
        if let Err(reason) = result {
            failures.push(format!(
                "{}: {reason}; {}",
                serde_json::to_value(gate)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                remedy.replace("<id>", &run.id)
            ));
        }
    }
    if !failures.is_empty() {
        return Err(format!("Cannot advance:\n{}", failures.join("\n")));
    }
    apply(run, transition.to);
    Ok(())
}

fn apply(run: &mut Run, to: Destination) {
    match to {
        Destination::Stage(stage) => run.stage = stage,
        Destination::Done => run.status = RunStatus::Done,
    }
}

/// The body of `POST /api/runs/<id>/advance`: an optional forced advance with the reason for it.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdvanceRequest {
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

impl AdvanceRequest {
    /// The reason for a forced advance, or `None` for a normal one.
    ///
    /// # Errors
    ///
    /// Returns a message when `force` and a nonempty `reason` do not come together.
    pub(crate) fn forced_reason(&self) -> Result<Option<&str>, String> {
        let reason = self.reason.as_deref().map(str::trim);
        match (self.force, reason) {
            (true, Some(reason)) if !reason.is_empty() => Ok(Some(reason)),
            (true, _) => Err("--force needs a nonempty --reason \"<text>\"".into()),
            (false, None) => Ok(None),
            (false, Some(_)) => Err("--reason is only for --force".into()),
        }
    }
}

/// Moves the run along the stage graph without checking Gates and records `reason` as a run report.
///
/// # Errors
///
/// Returns a message when the mode is unrecorded, the run cannot move, or the reason does not fit a report.
pub(crate) fn advance_forced(run: &mut Run, reason: &str) -> Result<(), String> {
    let mode = run.mode.ok_or("Record the run's mode first")?;
    let transition = stages::next(mode, run.stage, run.status)?;
    let text = format!("Forced advance from {}: {reason}", run.stage.name());
    if text.len() > MAX_TEXT_BYTES || text.chars().any(char::is_control) {
        return Err(format!(
            "The reason must be short ({MAX_TEXT_BYTES} bytes with the prefix) and contain no control characters"
        ));
    }
    if run.run_reports.len() >= MAX_REPORTS_PER_RUN {
        return Err(format!(
            "A run can hold at most {MAX_REPORTS_PER_RUN} run reports"
        ));
    }
    run.run_reports.push(RunReport {
        kind: ReportKind::Activity,
        text,
        stage: run.stage,
        at: updated_now().map_err(|error| format!("Could not read the time: {error}"))?,
    });
    apply(run, transition.to);
    Ok(())
}

fn require(condition: bool, reason: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(reason.to_string())
    }
}

fn verified(run: &Run, git: &dyn Git) -> Result<(), String> {
    let result = latest(run, Gate::Verified)
        .filter(|r| r.passed)
        .ok_or("No passing verification was recorded")?;
    let current = head(run, git)?;
    require(
        result.commit.as_deref() == Some(current.as_str()),
        "The verification commit does not match HEAD",
    )
}

fn ci_passed(
    run: &Run,
    git: &dyn Git,
    tool: &dyn pr_wait::Gh,
    clock: &dyn pr_wait::Clock,
) -> Result<(), String> {
    let url = run.pr.as_deref().ok_or("No pull request is recorded")?;
    let state = pr_state::read(tool, url)
        .map_err(|error| format!("Could not read pull request state: {}", error.0))?;
    require(
        state == pr_state::PrState::Open,
        &format!(
            "Pull request is {}; if it merged, run `jidoka run sync {}` to reconcile the run",
            state.label(),
            run.id
        ),
    )?;
    let commit = head(run, git)?;
    let options = pr_wait::Options {
        timeout: CI_TIMEOUT,
        pushed_head: None,
        ..Default::default()
    };
    let result = pr_wait::wait(Some(url), &options, tool, clock, &mut |_| {}, &mut |_| {})
        .map_err(|error| format!("Could not check CI: {}", error.0))?;
    if result.outcome == pr_wait::Outcome::NotOpen {
        return Err(format!(
            "Pull request closed or merged while checking CI; run `jidoka run sync {}` to reconcile the run",
            run.id
        ));
    }
    require(
        result.head == commit,
        "The pull request head does not match the worktree HEAD",
    )?;
    require(
        matches!(
            result.outcome,
            pr_wait::Outcome::Passed | pr_wait::Outcome::NoCi
        ),
        &format!("CI outcome is {:?}", result.outcome),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pr_wait::{GhError, GhOutput},
        run::{Mode, Ticket},
    };
    use std::cell::RefCell;

    struct FakeGit(RefCell<String>);
    impl Git for FakeGit {
        fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
            Ok(self.0.borrow().clone())
        }
    }
    struct FakeGh {
        state: &'static str,
        bucket: &'static str,
        head: &'static str,
    }
    impl pr_wait::Gh for FakeGh {
        fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
            let value = if args.contains(&"checks") {
                serde_json::json!([{"name":"test", "bucket":self.bucket, "state":"SUCCESS", "link":""}])
            } else if args.contains(&"state,isDraft") {
                serde_json::json!({"state":self.state,"isDraft":false})
            } else {
                serde_json::json!({"state":self.state,"headRefOid":self.head,"mergeStateStatus":"CLEAN","baseRefName":"main"})
            };
            Ok(GhOutput {
                code: Some(0),
                stdout: value.to_string(),
                stderr: String::new(),
            })
        }
    }
    fn run(stage: Stage) -> Run {
        let mut run: Run = serde_json::from_value(serde_json::json!({"gates":[],"id":"1","title":"t","project":"/p","stage":stage,"status":"running"})).unwrap();
        run.mode = Some(Mode::Fast);
        run.worktree = Some("/work".into());
        run.pr = Some("https://github.com/o/r/pull/7".into());
        run
    }
    fn result(gate: Gate, passed: bool) -> Record {
        Record {
            gate,
            passed,
            evidence: None,
        }
    }
    fn host() -> FakeGh {
        FakeGh {
            state: "OPEN",
            bucket: "pass",
            head: "abc1234",
        }
    }
    #[test]
    fn forced_advance_skips_gates_records_the_reason_and_needs_one() {
        let mut run = run(Stage::Build);
        advance_forced(&mut run, "Tickets done by hand").unwrap();
        assert_eq!(run.stage, Stage::Finish);
        assert_eq!(
            run.run_reports.last().unwrap().text,
            "Forced advance from build: Tickets done by hand"
        );
        let request = |force, reason: Option<&str>| AdvanceRequest {
            force,
            reason: reason.map(String::from),
        };
        assert!(request(true, None).forced_reason().is_err());
        assert!(request(true, Some(" ")).forced_reason().is_err());
        assert!(request(false, Some("why")).forced_reason().is_err());
        assert_eq!(
            request(true, Some("why")).forced_reason().unwrap(),
            Some("why")
        );
        assert_eq!(request(false, None).forced_reason().unwrap(), None);
    }

    #[test]
    fn build_requires_tickets_and_verification_of_current_head() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Build);
        run.tickets.push(Ticket {
            reference: "1".into(),
            title: "one".into(),
            status: TicketStatus::Pending,
        });
        let error = advance(&mut run, &git, &host()).unwrap_err();
        assert!(error.contains("tickets-done") && error.contains("verified"));
        record(&mut run, result(Gate::Verified, true), &git).unwrap();
        run.tickets[0].status = TicketStatus::Done;
        *git.0.borrow_mut() = "def5678".into();
        assert!(
            advance(&mut run, &git, &host())
                .unwrap_err()
                .contains("verified")
        );
        record(&mut run, result(Gate::Verified, false), &git).unwrap();
        assert!(advance(&mut run, &git, &host()).is_err());
        record(&mut run, result(Gate::Verified, true), &git).unwrap();
        advance(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Finish);
    }
    #[test]
    fn full_build_requires_a_passing_result_for_each_ticket() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Build);
        run.mode = Some(Mode::Full);
        for reference in ["one", "two"] {
            run.tickets.push(Ticket {
                reference: reference.into(),
                title: reference.into(),
                status: TicketStatus::Done,
            });
        }
        assert!(
            advance(&mut run, &git, &host())
                .unwrap_err()
                .contains("tickets-verified")
        );
        run.current_ticket = Some("one".into());
        record(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
        assert!(advance(&mut run, &git, &host()).is_err());
        run.current_ticket = Some("two".into());
        record(&mut run, result(Gate::TicketsVerified, false), &git).unwrap();
        assert!(advance(&mut run, &git, &host()).is_err());
        record(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
        advance(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Check);
    }

    #[test]
    fn full_run_walks_stages_and_rework_with_ticket_verification() {
        use crate::run::{RunStore, parse_update};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runs.json");
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut initial = run(Stage::Define);
        initial.mode = Some(Mode::Full);
        std::fs::write(&path, serde_json::to_vec(&vec![initial]).unwrap()).unwrap();
        let store = RunStore::open(&path).unwrap();
        let update = |options: &[&str]| {
            store
                .update("1", parse_update("running", options).unwrap())
                .unwrap()
                .unwrap();
        };
        let gate = |record| store.change_gate("1", |run| super::record(run, record, &git));
        let advance = || store.change_gate("1", |run| super::advance(run, &git, &host()));
        update(&[
            "--ticket",
            "https://example.com/1 One",
            "--ticket",
            "https://example.com/2 Two",
        ]);
        advance().unwrap();
        let current = store.get("1").unwrap().unwrap();
        let next = serde_json::to_value(crate::run_next::build(&current, &[]).unwrap()).unwrap();
        assert!(
            next["instructions"]
                .as_str()
                .unwrap()
                .contains("tickets-verified pass")
        );
        for reference in ["https://example.com/1", "https://example.com/2"] {
            update(&["--current-ticket", reference]);
            gate(result(Gate::TicketsVerified, true)).unwrap();
            update(&["--ticket-status", &format!("{reference}=done")]);
        }
        advance().unwrap();
        assert_eq!(store.get("1").unwrap().unwrap().stage, Stage::Check);
        update(&["--rework", "1"]);
        advance().unwrap();
        gate(result(Gate::ChecksRecorded, true)).unwrap();
        gate(result(Gate::NoActionableFindings, true)).unwrap();
        gate(result(Gate::Verified, true)).unwrap();
        advance().unwrap();
        advance().unwrap();
        assert_eq!(store.get("1").unwrap().unwrap().status, RunStatus::Done);
    }

    #[test]
    fn merged_pull_request_advise_sync_instead_of_accepting_unchecked_ci() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Finish);
        let error = advance(
            &mut run,
            &git,
            &FakeGh {
                state: "MERGED",
                ..host()
            },
        )
        .unwrap_err();
        assert!(error.contains("jidoka run sync 1"));
        assert_eq!(run.status, RunStatus::Running);
    }

    #[test]
    fn merge_during_ci_check_reports_sync_even_if_the_head_changed() {
        struct MergedDuringWait;
        impl pr_wait::Gh for MergedDuringWait {
            fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
                let value = if args.contains(&"state,isDraft") {
                    serde_json::json!({"state":"OPEN","isDraft":false})
                } else {
                    serde_json::json!({"state":"MERGED","headRefOid":"new-head","mergeStateStatus":"CLEAN","baseRefName":"main"})
                };
                Ok(GhOutput {
                    code: Some(0),
                    stdout: value.to_string(),
                    stderr: String::new(),
                })
            }
        }
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Finish);
        let error = advance(&mut run, &git, &MergedDuringWait).unwrap_err();
        assert!(error.contains("jidoka run sync 1"), "{error}");
        assert_eq!(run.status, RunStatus::Running);
    }

    #[test]
    fn finish_checks_pr_state_ci_and_head() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        for host in [
            FakeGh {
                state: "CLOSED",
                ..host()
            },
            FakeGh {
                bucket: "fail",
                ..host()
            },
            FakeGh {
                head: "def5678",
                ..host()
            },
        ] {
            assert!(advance(&mut run(Stage::Finish), &git, &host).is_err());
        }
        let mut run = run(Stage::Finish);
        advance(&mut run, &git, &host()).unwrap();
        assert_eq!(run.status, RunStatus::Done);
    }
    #[test]
    fn review_modes_verify_the_final_commit_after_review_and_rework() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        for mode in [Mode::Balanced, Mode::Full] {
            *git.0.borrow_mut() = "abc1234".into();
            let mut run = run(Stage::Build);
            run.mode = Some(mode);
            run.rework = Some(1);
            run.tickets.push(Ticket {
                reference: "ticket".into(),
                title: "ticket".into(),
                status: TicketStatus::Done,
            });
            if mode == Mode::Full {
                run.current_ticket = Some("ticket".into());
                record(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
            }
            advance(&mut run, &git, &host()).unwrap();
            assert_eq!(run.stage, Stage::Check);
            record(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
            record(&mut run, result(Gate::NoActionableFindings, true), &git).unwrap();
            assert!(
                advance(&mut run, &git, &host())
                    .unwrap_err()
                    .contains("verified")
            );
            record(&mut run, result(Gate::Verified, true), &git).unwrap();
            *git.0.borrow_mut() = "def5678".into();
            assert!(
                advance(&mut run, &git, &host())
                    .unwrap_err()
                    .contains("verified")
            );
            record(&mut run, result(Gate::Verified, true), &git).unwrap();
            advance(&mut run, &git, &host()).unwrap();
            assert_eq!(run.stage, Stage::Finish);
        }
    }
    #[derive(Default)]
    struct Clock(std::cell::Cell<Duration>);
    impl pr_wait::Clock for Clock {
        fn elapsed(&self) -> Duration {
            self.0.get()
        }
        fn sleep(&self, duration: Duration) {
            self.0.set(self.0.get() + duration);
        }
    }

    struct AppearingChecks {
        answers: RefCell<std::collections::VecDeque<&'static str>>,
    }
    impl pr_wait::Gh for AppearingChecks {
        fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
            if args.contains(&"checks") {
                let mut answers = self.answers.borrow_mut();
                let answer = if answers.len() > 1 {
                    answers.pop_front().unwrap()
                } else {
                    *answers.front().unwrap()
                };
                return Ok(GhOutput {
                    code: Some(0),
                    stdout: answer.into(),
                    stderr: String::new(),
                });
            }
            host().run(args)
        }
    }

    #[test]
    fn no_ci_waits_for_the_grace_period_and_new_failures_block_advance() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let clock = Clock::default();
        let no_checks = AppearingChecks {
            answers: RefCell::new(["[]"].into()),
        };
        let mut no_ci_run = run(Stage::Finish);
        advance_with_clock(&mut no_ci_run, &git, &no_checks, &clock).unwrap();
        assert!(clock.0.get() >= Duration::from_secs(90));
        assert_eq!(no_ci_run.status, RunStatus::Done);

        let clock = Clock::default();
        let late_failure = AppearingChecks {
            answers: RefCell::new(
                [
                    "[]",
                    r#"[{"name":"build","bucket":"fail","state":"FAILURE","link":""}]"#,
                ]
                .into(),
            ),
        };
        let mut run = run(Stage::Finish);
        assert!(advance_with_clock(&mut run, &git, &late_failure, &clock).is_err());
        assert_eq!(run.status, RunStatus::Running);
        assert!(clock.0.get() < Duration::from_secs(90));
    }

    struct BrokenGit;
    impl Git for BrokenGit {
        fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
            Err("repository unavailable".into())
        }
    }
    struct BrokenHost;
    impl pr_wait::Gh for BrokenHost {
        fn run(&self, _: &[&str]) -> Result<GhOutput, GhError> {
            Err(GhError::Other("host unavailable".into()))
        }
    }
    #[test]
    fn failed_external_checks_report_the_cause_and_remedy() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut build = run(Stage::Build);
        record(&mut build, result(Gate::Verified, true), &git).unwrap();
        let error = advance(&mut build, &BrokenGit, &host()).unwrap_err();
        assert!(
            error.contains("repository unavailable") && error.contains("jidoka run gate"),
            "{error}"
        );
        let error = advance(&mut run(Stage::Finish), &git, &BrokenHost).unwrap_err();
        assert!(
            error.contains("host unavailable") && error.contains("jidoka pr wait"),
            "{error}"
        );
    }

    struct Azure;
    impl pr_wait::Gh for Azure {
        fn run(&self, _: &[&str]) -> Result<GhOutput, GhError> {
            panic!("Azure must not use gh")
        }
        fn az(&self, args: &[&str]) -> Result<GhOutput, GhError> {
            let value = match args {
                ["repos", "pr", "show", ..] => {
                    serde_json::json!({"pullRequestId":535,"status":"active","isDraft":false,"sourceRefName":"refs/heads/feature","lastMergeSourceCommit":{"commitId":"abc1234"},"repository":{"id":"repo","name":"Repo","project":{"name":"Project"}}})
                }
                ["repos", "ref", "list", ..] => {
                    serde_json::json!([{"name":"refs/heads/feature","objectId":"abc1234"}])
                }
                ["repos", "pr", "policy", "list", ..] => {
                    serde_json::json!([{"status":"approved","startedDate":"2026-01-01T00:00:01Z","configuration":{"isEnabled":true,"isBlocking":true,"type":{"displayName":"Build"}}}])
                }
                ["devops", "invoke", ..] if args.contains(&"pullRequestIterations") => {
                    serde_json::json!({"value":[{"id":1,"createdDate":"2026-01-01T00:00:00Z","sourceRefCommit":{"commitId":"abc1234"}}]})
                }
                ["devops", "invoke", ..] => serde_json::json!({"value":[]}),
                _ => panic!("unexpected Azure command {args:?}"),
            };
            Ok(GhOutput {
                code: Some(0),
                stdout: value.to_string(),
                stderr: String::new(),
            })
        }
    }
    #[test]
    fn azure_ci_can_pass() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Finish);
        run.pr = Some("https://dev.azure.com/org/Project/_git/Repo/pullrequest/535".into());
        advance(&mut run, &git, &Azure).unwrap();
        assert_eq!(run.status, RunStatus::Done);
    }
    #[test]
    fn check_requires_recorded_passing_results() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        record(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
        record(&mut run, result(Gate::NoActionableFindings, false), &git).unwrap();
        assert!(advance(&mut run, &git, &host()).is_err());
        record(&mut run, result(Gate::NoActionableFindings, true), &git).unwrap();
        record(&mut run, result(Gate::Verified, true), &git).unwrap();
        advance(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Finish);
    }
}
