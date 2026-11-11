//! Gate evidence and stage transitions.
use crate::{
    pr_state,
    pr_sync::Git,
    pr_wait,
    run::{Run, RunStatus, Stage, TicketStatus},
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
                "mark every ticket done with `jidoka run update <id> build running --ticket-status <ref>=done`",
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
                "set the current ticket with `jidoka run update <id> build running --current-ticket <reference>`, verify it, then run `jidoka run gate <id> tickets-verified pass`",
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
    match transition.to {
        Destination::Stage(stage) => run.stage = stage,
        Destination::Done => run.status = RunStatus::Done,
    }
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
        &format!("Pull request is {}", state.label()),
    )?;
    let commit = head(run, git)?;
    let options = pr_wait::Options {
        timeout: CI_TIMEOUT,
        pushed_head: None,
        ..Default::default()
    };
    let result = pr_wait::wait(Some(url), &options, tool, clock, &mut |_| {}, &mut |_| {})
        .map_err(|error| format!("Could not check CI: {}", error.0))?;
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
