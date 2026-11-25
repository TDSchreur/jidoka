use super::*;
use crate::{
    cli::Command,
    pr_host::{FakeHost, Head},
    pr_state,
    pr_wait::Outcome,
    run::{
        AdvanceMetadata, CiOutcome, Completion, ExitOutcome, GateStatus, Mode, RunStatus, Stage,
        Ticket, TicketStatus,
    },
};
use request::AdvanceOptions;
use std::{cell::RefCell, path::Path};

/// Records a gate on a run value, as the store does with the result.
fn record_on(run: &mut Run, gate: Record, git: &dyn Git) -> Result<(), String> {
    let result = record(run, gate, git)?;
    run.gates.push(result);
    Ok(())
}

/// Leaves the Stage of a run value once its exit gates pass, as the store does with the exit.
fn advance_on(run: &mut Run, git: &dyn Git, tool: &dyn PrHost) -> Result<(), String> {
    let exit = accepted_exit(run, git, tool)?;
    run.leave(exit, 1)
}

/// Advances run 1 through the store once its exit gates pass.
fn advance_stored(
    store: &crate::run::RunStore,
    git: &dyn Git,
    tool: &dyn PrHost,
) -> Result<(), String> {
    store.advance("1", AdvanceMetadata::default(), |run| {
        accepted_exit(run, git, tool)
    })
}

fn force(run: &mut Run, reason: &str) -> Result<(), String> {
    run.leave(
        Exit::Override {
            reason: reason.into(),
        },
        1,
    )
}

struct FakeGit(RefCell<String>);
impl Git for FakeGit {
    fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
        Ok(self.0.borrow().clone())
    }

    fn status(&self, _: &Path, args: &[&str]) -> Result<i32, String> {
        Ok(match args {
            ["merge-base", "--is-ancestor", "abc1234", "def5678"] => 0,
            ["merge-base", "--is-ancestor", _, _] => 1,
            _ => panic!("Unexpected git status command {args:?}"),
        })
    }
}
/// A running Fast run in `stage` with a worktree, base commit, pull request and Finish summary.
pub(super) fn run(stage: Stage) -> Run {
    let mut run: Run = serde_json::from_value(serde_json::json!({"gates":[],"id":"1","title":"t","project":"/p","stage":stage,"status":"running"})).unwrap();
    run.mode = Some(Mode::Fast);
    run.worktree = Some("/work".into());
    run.base_commit = Some("abc1234".into());
    run.pr = Some("https://github.com/o/r/pull/7".into());
    run.summaries
        .insert(Stage::Finish, "Implemented the task".into());
    run
}
fn result(gate: Gate, passed: bool) -> Record {
    Record {
        gate,
        passed,
        evidence: None,
        check: (gate == Gate::ChecksRecorded).then(|| "code-review".to_owned()),
    }
}
/// An open pull request at commit `abc1234` whose CI passes.
fn host() -> FakeHost {
    FakeHost::states(&[pr_state::PrState::Open])
}

/// An open pull request at `commit` whose CI passes.
fn host_at(state: pr_state::PrState, commit: &str) -> FakeHost {
    FakeHost::new(vec![Ok(Head {
        state,
        commit: commit.into(),
        branch: "feature".into(),
    })])
}

#[test]
fn advance_keeps_usage_collected_meanwhile_but_refuses_other_changes() {
    use crate::run::RunStore;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runs.json");
    std::fs::write(&path, serde_json::to_vec(&vec![run(Stage::Build)]).unwrap()).unwrap();
    let store = RunStore::open(&path).unwrap();
    let usage: crate::usage::token_usage::Usage =
        serde_json::from_value(serde_json::json!({"status":"tracked","collected_at":7})).unwrap();
    let forced = || Exit::Override {
        reason: "Tickets done by hand".into(),
    };
    store
        .advance("1", AdvanceMetadata::default(), |_| {
            store.set_usage("1", usage.clone()).unwrap();
            Ok(forced())
        })
        .unwrap();
    let saved = store.get("1").unwrap().unwrap();
    assert_eq!(saved.stage, Stage::Finish);
    assert_eq!(saved.usage, Some(usage));
    assert!(!saved.run_reports.is_empty());

    let refused = store.advance("1", AdvanceMetadata::default(), |_| {
        let update = crate::run::parse_update(&["--activity", "Fixing CI"]).unwrap();
        store.update("1", update).unwrap();
        Ok(forced())
    });
    assert_eq!(
        refused.unwrap_err(),
        "The run changed while checking gates; try again"
    );
    let kept = store.get("1").unwrap().unwrap();
    assert_eq!(
        (kept.stage, kept.status),
        (Stage::Finish, RunStatus::Running)
    );
    assert_eq!(kept.activity.as_deref(), Some("Fixing CI"));
}

#[test]
fn forced_advance_skips_gates_records_the_reason_and_needs_one() {
    let mut run = run(Stage::Build);
    force(&mut run, "Tickets done by hand").unwrap();
    assert_eq!(run.stage, Stage::Finish);
    assert_eq!(
        run.run_reports.last().unwrap().text,
        "Forced advance from build: Tickets done by hand"
    );
    let request = |force, reason: Option<&str>| AdvanceRequest {
        force,
        reason: reason.map(String::from),
        ..AdvanceRequest::default()
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
fn advance_options_are_read_in_any_order_and_refused_when_wrong() {
    let parse_advance_options = |options: &[&str]| {
        let words = [&["1"][..], options].concat();
        AdvanceCommand::parse(&words).map(|(_, parsed)| parsed)
    };
    let parsed = parse_advance_options(&[
        "--json",
        "--activity",
        "Checking",
        "--summary",
        "build=Done",
    ])
    .unwrap();
    assert!(parsed.json && !parsed.request.force);
    let body = serde_json::to_value(parsed.request).unwrap();
    assert_eq!(body["activity"], "Checking");
    assert_eq!(body["summaries"]["build"], "Done");
    assert_eq!(
        parse_advance_options(&["--force", "--reason", "why"])
            .unwrap()
            .request,
        AdvanceRequest {
            force: true,
            reason: Some("why".into()),
            ..AdvanceRequest::default()
        }
    );
    assert_eq!(
        parse_advance_options(&[]).unwrap(),
        AdvanceOptions::default()
    );
    for bad in [
        &["--bogus"][..],
        &["--activity"],
        &["--json", "--json"],
        &["--force"],
        &["--reason", "why"],
        &["--summary", "a=b", "--summary", "c=d"],
    ] {
        assert!(parse_advance_options(bad).is_err(), "{bad:?}");
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
        work_attempt: 0,
    });
    let error = advance_on(&mut run, &git, &host()).unwrap_err();
    assert!(error.contains("tickets-done") && error.contains("verified"));
    record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
    run.tickets[0].status = TicketStatus::Done;
    *git.0.borrow_mut() = "def5678".into();
    assert!(
        advance_on(&mut run, &git, &host())
            .unwrap_err()
            .contains("verified")
    );
    record_on(&mut run, result(Gate::Verified, false), &git).unwrap();
    assert!(advance_on(&mut run, &git, &host()).is_err());
    record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
    advance_on(&mut run, &git, &host()).unwrap();
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
            status: TicketStatus::Pending,
            work_attempt: 0,
        });
    }
    assert!(
        advance_on(&mut run, &git, &host())
            .unwrap_err()
            .contains("tickets-verified")
    );
    assert!(record_on(&mut run, result(Gate::TicketsVerified, true), &git).is_err());
    run.tickets[0].status = TicketStatus::InProgress;
    record_on(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
    run.tickets[0].status = TicketStatus::Done;
    assert!(advance_on(&mut run, &git, &host()).is_err());
    run.tickets[1].status = TicketStatus::InProgress;
    record_on(&mut run, result(Gate::TicketsVerified, false), &git).unwrap();
    assert!(advance_on(&mut run, &git, &host()).is_err());
    record_on(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
    run.tickets[1].status = TicketStatus::Done;
    advance_on(&mut run, &git, &host()).unwrap();
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
            .update_with("1", parse_update(options).unwrap(), &git)
            .unwrap()
            .unwrap();
    };
    let gate = |result| store.record_gate("1", |run| record(run, result, &git));
    let gh = host_at(pr_state::PrState::Open, "def5678");
    let advance = || advance_stored(&store, &git, &gh);
    update(&[
        "--ticket",
        "https://example.com/1 One",
        "--ticket",
        "https://example.com/2 Two",
    ]);
    advance().unwrap();
    let current = store.get("1").unwrap().unwrap();
    let next = serde_json::to_value(crate::run_next::build(&current, &git).unwrap()).unwrap();
    assert!(
        next["instructions"]
            .as_str()
            .unwrap()
            .contains("tickets-verified pass")
    );
    for (reference, commit) in [
        ("https://example.com/1", "abc1234"),
        ("https://example.com/2", "def5678"),
    ] {
        *git.0.borrow_mut() = commit.into();
        update(&["--ticket-status", &format!("{reference}=in-progress")]);
        gate(result(Gate::TicketsVerified, true)).unwrap();
        let current = store.get("1").unwrap().unwrap();
        let active = current.active_ticket().unwrap();
        assert_eq!(
            current.timeline.last().unwrap().ticket.as_deref(),
            Some(reference)
        );
        let verified = latest_ticket_verification(&current, active).unwrap();
        assert_eq!(verified.commit.as_deref(), Some(commit));
        update(&["--ticket-status", &format!("{reference}=done")]);
    }
    *git.0.borrow_mut() = "other123".into();
    let error = advance().unwrap_err().to_string();
    assert!(error.contains("outside the current Git history"), "{error}");
    *git.0.borrow_mut() = "def5678".into();
    advance().unwrap();
    let first_build = store.get("1").unwrap().unwrap().stage_exits[1].clone();
    update(&["--finding", "code-review", "-", "Bug"]);
    store
        .transition(
            "1",
            crate::run::parse_lifecycle("rework", &["--finding", "F1=behavior"]).unwrap(),
        )
        .unwrap();
    let reopened = "https://example.com/2";
    update(&["--ticket-status", &format!("{reopened}=in-progress")]);
    update(&["--ticket-status", &format!("{reopened}=done")]);
    let error = advance().unwrap_err().to_string();
    assert!(
        error.contains(reopened) && error.contains("current attempt"),
        "{error}"
    );
    update(&["--ticket-status", &format!("{reopened}=in-progress")]);
    gate(result(Gate::TicketsVerified, true)).unwrap();
    update(&["--ticket-status", &format!("{reopened}=done")]);
    advance().unwrap();
    gate(result(Gate::ChecksRecorded, true)).unwrap();
    let error = advance().unwrap_err();
    assert!(error.contains("F1 are open"), "{error}");
    assert!(gate(result(Gate::NoActionableFindings, true)).is_err());
    update(&["--finding-status", "F1=fixed"]);
    gate(result(Gate::NoActionableFindings, true)).unwrap();
    gate(result(Gate::Verified, true)).unwrap();
    advance().unwrap();
    advance().unwrap();
    let finished = store.get("1").unwrap().unwrap();
    assert_eq!(finished.status, RunStatus::Done);
    assert_eq!(finished.stage_exits[1], first_build);
    assert!(matches!(
        finished.stage_exits[2].outcome,
        ExitOutcome::Rework { number: 1, .. }
    ));
    assert_ne!(finished.stage_exits[1].gates, finished.stage_exits[3].gates);
}

#[test]
fn a_finding_opened_after_review_blocks_next_and_advance() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Check);
    run.mode = Some(Mode::Balanced);
    for gate in [
        Gate::ChecksRecorded,
        Gate::NoActionableFindings,
        Gate::Verified,
    ] {
        record_on(&mut run, result(gate, true), &git).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.json");
    std::fs::write(&path, serde_json::to_vec(&[&run]).unwrap()).unwrap();
    let store = crate::run::RunStore::open(&path).unwrap();
    let update = crate::run::parse_update(&["--finding", "code-review", "-", "Bug"]).unwrap();
    let run = store.update_with("1", update, &git).unwrap().unwrap();
    let next = crate::run_next::build(&run, &git).unwrap();
    let text = crate::run_next::render(&next);
    assert!(
        text.contains("Findings F1 are open"),
        "Missing open Finding in guidance: {text}"
    );
    assert!(
        text.contains("Move to build (rework)"),
        "Missing Rework move in guidance: {text}"
    );
    let dashboard = serde_json::to_value(crate::run_next::stage_gates(
        &run,
        assess_local(&run, &git, AssessmentPurpose::Dashboard).evaluations,
    ))
    .unwrap();
    assert!(dashboard.to_string().contains("Findings F1 are open"));
    let error = advance_stored(&store, &git, &host()).unwrap_err();
    assert!(error.contains("Findings F1 are open"));
    assert!(store.get("1").unwrap().unwrap().stage_exits.is_empty());
}

#[test]
fn named_gate_failures_show_their_evidence_for_the_current_pass() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    for (stage, gate, subject) in [
        (Stage::Check, Gate::ChecksRecorded, "code-review"),
        (Stage::Build, Gate::TicketsVerified, "local-1"),
    ] {
        let mut run = run(stage);
        run.mode = Some(Mode::Full);
        if stage == Stage::Build {
            run.tickets.push(Ticket {
                reference: subject.into(),
                title: "Work".into(),
                status: TicketStatus::InProgress,
                work_attempt: 1,
            });
        }
        let mut failed = result(gate, false);
        failed.evidence = Some("Verification timed out".into());
        record_on(&mut run, failed, &git).unwrap();
        let views = |run: &Run| {
            serde_json::to_value(crate::run_next::stage_gates(
                run,
                assess_local(run, &git, AssessmentPurpose::Dashboard).evaluations,
            ))
            .unwrap()
        };
        let evaluated = |views: serde_json::Value| {
            views
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["stage"] == stage.name())
                .unwrap()["gates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|result| result["name"] == gate.name())
                .unwrap()
                .clone()
        };
        let result = evaluated(views(&run));
        assert_eq!(result["status"], "failed");
        let field = if stage == Stage::Build {
            "ticket"
        } else {
            "check"
        };
        assert_eq!(result["results"][0][field], subject);
        assert_eq!(result["results"][0]["evidence"], "Verification timed out");
        if stage == Stage::Build {
            run.tickets[0].work_attempt += 1;
        } else {
            run.rework = Some(1);
        }
        assert_eq!(evaluated(views(&run))["status"], "open");
    }
}

#[test]
fn full_unticketed_build_requires_a_declaration_and_a_pass_on_the_current_head() {
    use crate::run::{RunStore, parse_update};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runs.json");
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut initial = run(Stage::Build);
    initial.mode = Some(Mode::Full);
    std::fs::write(&path, serde_json::to_vec(&vec![initial]).unwrap()).unwrap();
    let store = RunStore::open(&path).unwrap();
    let update = |options: &[&str]| {
        store
            .update_with("1", parse_update(options).unwrap(), &git)
            .unwrap();
    };
    let advance = || advance_stored(&store, &git, &host());
    let pass = || {
        store.record_gate("1", |run| {
            record(run, result(Gate::TicketsVerified, true), &git)
        })
    };

    let error = advance().unwrap_err().to_string();
    assert!(
        error.contains("tickets-done") && error.contains("no ticket plan"),
        "{error}"
    );
    let error = pass().unwrap_err();
    assert!(
        error.contains("Mark a listed ticket in progress")
            && error.contains("declare unticketed work"),
        "{error}"
    );
    update(&["--unticketed"]);
    let error = advance().unwrap_err().to_string();
    assert!(
        error.contains("tickets-verified") && !error.contains("tickets-done"),
        "{error}"
    );
    pass().unwrap();
    *git.0.borrow_mut() = "def5678".into();
    assert!(advance().is_err(), "a moved HEAD needs fresh evidence");
    pass().unwrap();
    update(&["--clear-tickets"]);
    update(&["--unticketed"]);
    assert!(advance().is_err(), "a replaced plan needs fresh evidence");
    pass().unwrap();
    advance().unwrap();
}

#[test]
fn merged_pull_request_advise_sync_instead_of_accepting_unchecked_ci() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Finish);
    let error = advance_on(
        &mut run,
        &git,
        &FakeHost::states(&[pr_state::PrState::Merged]),
    )
    .unwrap_err();
    assert!(error.contains("jidoka run sync 1"));
    assert_eq!(run.status, RunStatus::Running);
}

#[test]
fn merge_during_ci_check_reports_sync_even_if_the_head_changed() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Finish);
    let merged = host_at(pr_state::PrState::Merged, "new-head");
    let error = advance_on(&mut run, &git, &merged).unwrap_err();
    assert!(error.contains("jidoka run sync 1"), "{error}");
    assert_eq!(run.status, RunStatus::Running);
}

#[test]
fn finish_checks_pr_state_ci_and_head() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    for host in [
        FakeHost::states(&[pr_state::PrState::Closed]),
        host().with_ci(Outcome::Failed),
        host_at(pr_state::PrState::Open, "def5678"),
    ] {
        assert!(advance_on(&mut run(Stage::Finish), &git, &host).is_err());
    }
    let mut run = run(Stage::Finish);
    advance_on(&mut run, &git, &host()).unwrap();
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
            work_attempt: 0,
        });
        if mode == Mode::Full {
            run.tickets[0].status = TicketStatus::InProgress;
            record_on(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
            run.tickets[0].status = TicketStatus::Done;
        }
        advance_on(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Check);
        record_on(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
        record_on(&mut run, result(Gate::NoActionableFindings, true), &git).unwrap();
        assert!(
            advance_on(&mut run, &git, &host())
                .unwrap_err()
                .contains("verified")
        );
        record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
        *git.0.borrow_mut() = "def5678".into();
        assert!(
            advance_on(&mut run, &git, &host())
                .unwrap_err()
                .contains("verified")
        );
        record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
        assert!(
            advance_on(&mut run, &git, &host())
                .unwrap_err()
                .contains("checks-recorded")
        );
        record_on(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
        advance_on(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Finish);
    }
}
#[test]
fn finish_requires_a_summary_even_when_forced() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Finish);
    run.summaries.insert(Stage::Finish, "  ".into());
    assert!(
        advance_on(&mut run, &git, &host())
            .unwrap_err()
            .contains("--summary finish=")
    );
    assert!(force(&mut run, "CI service unavailable").is_err());
    assert_eq!(run.status, RunStatus::Running);
    assert!(run.run_reports.is_empty());
    assert!(run.gates.is_empty());

    run.summaries
        .insert(Stage::Finish, "Implemented the task".into());
    force(&mut run, "CI service unavailable").unwrap();
    assert_eq!(run.status, RunStatus::Done);
    assert_eq!(
        run.completion,
        Some(Completion::Override {
            reason: "CI service unavailable".into()
        })
    );
    assert!(run.gates.is_empty());
}

#[test]
fn advancing_out_of_finish_stores_the_ci_result() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runs.json");
    let initial = run(Stage::Finish);
    std::fs::write(&path, serde_json::to_vec(&vec![initial]).unwrap()).unwrap();
    let store = crate::run::RunStore::open(&path).unwrap();
    advance_stored(&store, &git, &host()).unwrap();
    let run = store.get("1").unwrap().unwrap();
    let stored = run.gates.iter().find(|r| r.gate == Gate::CiPassed).unwrap();
    assert!(stored.passed);
    assert_eq!(stored.stage, Stage::Finish);
    assert_eq!(stored.evidence.as_deref(), Some("CI passed on abc1234"));
    assert_eq!(
        run.completion,
        Some(Completion::Ci {
            head: "abc1234".into(),
            outcome: CiOutcome::Passed
        })
    );
    assert_eq!(
        run.pr_status.as_ref().map(|status| status.state),
        Some(pr_state::PrState::Open)
    );
    assert!(crate::inspect::render(&run).contains("CI passed on abc1234"));
    assert!(crate::inspect::render(&run).contains("PR state      open"));
    let mut merged = run.pr_status.clone().unwrap();
    merged.state = pr_state::PrState::Merged;
    store.record_pr_result("1", merged).unwrap();
    let reopened = crate::run::RunStore::open(&path).unwrap();
    let after_merge = reopened.get("1").unwrap().unwrap();
    assert_eq!(after_merge.completion, run.completion);
    assert_eq!(after_merge.stage_exits, run.stage_exits);
    assert_eq!(after_merge.finished_at, run.finished_at);
    assert_eq!(
        after_merge.pr_status.unwrap().state,
        pr_state::PrState::Merged
    );
}

#[test]
fn finishing_without_ci_records_no_ci() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Finish);
    advance_on(&mut run, &git, &host().with_ci(Outcome::NoCi)).unwrap();
    assert_eq!(run.status, RunStatus::Done);
    assert_eq!(
        run.completion,
        Some(Completion::Ci {
            head: "abc1234".into(),
            outcome: CiOutcome::NoCi
        })
    );
}

struct BrokenGit;
impl Git for BrokenGit {
    fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
        Err("repository unavailable".into())
    }
}
#[test]
fn failed_external_checks_report_the_cause_and_remedy() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut build = run(Stage::Build);
    record_on(&mut build, result(Gate::Verified, true), &git).unwrap();
    let error = advance_on(&mut build, &BrokenGit, &host()).unwrap_err();
    assert!(
        error.contains("repository unavailable") && error.contains("jidoka run gate"),
        "{error}"
    );
    let broken = FakeHost::new(vec![Err("host unavailable".into())]);
    let error = advance_on(&mut run(Stage::Finish), &git, &broken).unwrap_err();
    assert!(
        error.contains("host unavailable") && error.contains("jidoka pr wait"),
        "{error}"
    );
}

#[test]
fn check_requires_recorded_passing_results() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Check);
    run.mode = Some(Mode::Balanced);
    record_on(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
    record_on(&mut run, result(Gate::NoActionableFindings, false), &git).unwrap();
    assert!(advance_on(&mut run, &git, &host()).is_err());
    record_on(&mut run, result(Gate::NoActionableFindings, true), &git).unwrap();
    record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
    advance_on(&mut run, &git, &host()).unwrap();
    assert_eq!(run.stage, Stage::Finish);
}
#[test]
fn gate_options_accept_either_order_and_reject_duplicates() {
    let parse = |options: &[&str]| {
        let words = [&["1", "checks-recorded", "pass"][..], options].concat();
        GateCommand::parse(&words).map(|(_, record)| (record.check, record.evidence))
    };
    let expected = (Some("code-review".into()), Some("reviewed".into()));
    assert_eq!(
        parse(&["--check", "code-review", "--evidence", "reviewed"]).unwrap(),
        expected
    );
    assert_eq!(
        parse(&["--evidence", "reviewed", "--check", "code-review"]).unwrap(),
        expected
    );
    assert!(parse(&["--check", "a", "--check", "b"]).is_err());
}

#[test]
fn recording_requires_an_applicable_named_check() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Check);
    run.mode = Some(Mode::Balanced);
    let mut gate = result(Gate::ChecksRecorded, true);
    gate.check = None;
    assert!(
        record_on(&mut run, gate, &git)
            .unwrap_err()
            .contains("--check")
    );
    let mut gate = result(Gate::ChecksRecorded, true);
    gate.check = Some("nonexistent".into());
    assert!(
        record_on(&mut run, gate, &git)
            .unwrap_err()
            .contains("not applicable")
    );
}
#[test]
fn stale_check_evidence_blocks_guidance_and_advance() {
    let git = FakeGit(RefCell::new("abc1234".into()));
    let mut run = run(Stage::Check);
    run.mode = Some(Mode::Balanced);
    for gate in [
        Gate::ChecksRecorded,
        Gate::NoActionableFindings,
        Gate::Verified,
    ] {
        record_on(&mut run, result(gate, true), &git).unwrap();
    }
    let next = crate::run_next::build(&run, &git).unwrap();
    let next_json = serde_json::to_value(next).unwrap();
    let gates = &next_json["gates"];
    assert!(
        gates.as_array().unwrap().is_empty(),
        "Expected fresh evidence to satisfy every Gate, found {gates}"
    );

    *git.0.borrow_mut() = "def5678".into();
    let next = serde_json::to_value(crate::run_next::build(&run, &git).unwrap()).unwrap();
    let dashboard = assess_local(&run, &git, AssessmentPurpose::Dashboard);
    let error = advance_on(&mut run, &git, &host()).unwrap_err();
    for (gate, reason) in [
        (
            Gate::ChecksRecorded,
            "Check code-review has no passing result on HEAD",
        ),
        (
            Gate::Verified,
            "The verification commit does not match HEAD",
        ),
    ] {
        assert!(
            next["gates"].as_array().unwrap().iter().any(|entry| {
                entry["name"] == gate.name()
                    && entry["status"] == "open"
                    && entry["reason"] == reason
            }),
            "Missing {gate:?} in guidance: {next}"
        );
        assert!(
            dashboard.evaluations.iter().any(|entry| {
                entry.name == gate
                    && entry.status == GateStatus::Open
                    && entry.reason.as_deref() == Some(reason)
            }),
            "Missing {gate:?} in dashboard assessment"
        );
        assert!(
            error.contains(&format!("{}: {reason}", gate.name())),
            "{error}"
        );
    }
    assert_eq!(run.stage, Stage::Check);
    assert!(
        run.stage_exits.is_empty(),
        "Refused Advance saved Stage exits: {:?}",
        run.stage_exits
    );
}

#[test]
fn advance_rejects_head_change_during_check_selection() {
    struct MovingHead(std::cell::Cell<u32>);
    impl Git for MovingHead {
        fn git(&self, _: &Path, args: &[&str]) -> Result<String, String> {
            if args.starts_with(&["rev-parse", "HEAD"]) {
                let count = self.0.get();
                self.0.set(count + 1);
                Ok(if count == 0 { "old" } else { "new" }.into())
            } else {
                Ok(String::new())
            }
        }
    }
    let steady = FakeGit(RefCell::new("old".into()));
    let mut run = run(Stage::Check);
    run.mode = Some(Mode::Balanced);
    record_on(&mut run, result(Gate::ChecksRecorded, true), &steady).unwrap();
    record_on(&mut run, result(Gate::NoActionableFindings, true), &steady).unwrap();
    record_on(&mut run, result(Gate::Verified, true), &steady).unwrap();
    let error = advance_on(&mut run, &MovingHead(std::cell::Cell::new(0)), &host()).unwrap_err();
    assert!(error.contains("HEAD changed"), "{error}");
    assert_eq!(run.stage, Stage::Check);
    assert!(run.stage_exits.is_empty());
}

#[test]
fn independent_gate_records_merge_but_run_changes_reject_stale_validation() {
    use crate::run::{LifecycleRequest, RunStore, parse_update};
    struct Interleaved<'a> {
        store: &'a RunStore,
        id: &'a str,
        change_run: bool,
    }
    impl crate::git::Git for Interleaved<'_> {
        fn git(&self, _: &std::path::Path, _: &[&str]) -> Result<String, String> {
            if self.change_run {
                self.store
                    .transition(
                        self.id,
                        LifecycleRequest::Block {
                            reason: "waiting".into(),
                        },
                    )
                    .unwrap();
            } else {
                self.store
                    .record_gate(self.id, |run| {
                        record(
                            run,
                            Record {
                                gate: Gate::NoActionableFindings,
                                passed: true,
                                evidence: None,
                                check: None,
                            },
                            &crate::git::RealGit,
                        )
                    })
                    .unwrap();
            }
            Ok("abc123".into())
        }
    }
    let dir = crate::test_support::TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = crate::test_support::new_run(&store, "task");
    store
        .update(
            &run.id,
            parse_update(&["--worktree", "/work/repo"]).unwrap(),
        )
        .unwrap();
    let verified = || Record {
        gate: Gate::Verified,
        passed: true,
        evidence: None,
        check: None,
    };
    store
        .record_gate(&run.id, |run| {
            record(
                run,
                verified(),
                &Interleaved {
                    store: &store,
                    id: &run.id,
                    change_run: false,
                },
            )
        })
        .unwrap();
    let current = store.get(&run.id).unwrap().unwrap();
    assert_eq!(current.gates.len(), 2);
    assert!(
        current
            .gates
            .iter()
            .any(|gate| gate.gate == Gate::Verified && gate.commit.as_deref() == Some("abc123")),
        "verified gate should retain its checked commit"
    );
    assert!(
        current
            .gates
            .iter()
            .any(|gate| gate.gate == Gate::NoActionableFindings),
        "independent gate should be preserved"
    );
    let error = store
        .record_gate(&run.id, |run| {
            record(
                run,
                verified(),
                &Interleaved {
                    store: &store,
                    id: &run.id,
                    change_run: true,
                },
            )
        })
        .unwrap_err();
    assert!(
        error.contains("changed while checking"),
        "unexpected error: {error}"
    );
    assert_eq!(store.get(&run.id).unwrap().unwrap().gates.len(), 2);
}

#[test]
fn gate_record_rejects_run_change_reverted_during_validation() {
    use crate::run::{RunStore, parse_update};

    struct Revert<'a> {
        store: &'a RunStore,
        id: &'a str,
    }
    impl crate::git::Git for Revert<'_> {
        fn git(&self, _: &std::path::Path, _: &[&str]) -> Result<String, String> {
            for activity in ["B", "A"] {
                self.store
                    .update(self.id, parse_update(&["--activity", activity]).unwrap())
                    .unwrap();
            }
            Ok("abc123".into())
        }
    }
    let dir = crate::test_support::TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = crate::test_support::new_run(&store, "task");
    store
        .update(
            &run.id,
            parse_update(&["--worktree", "/work/repo", "--activity", "A"]).unwrap(),
        )
        .unwrap();
    let error = store
        .record_gate(&run.id, |run| {
            record(
                run,
                Record {
                    gate: Gate::Verified,
                    passed: true,
                    evidence: None,
                    check: None,
                },
                &Revert {
                    store: &store,
                    id: &run.id,
                },
            )
        })
        .unwrap_err();
    assert!(
        error.contains("changed while checking"),
        "unexpected error: {error}"
    );
    assert!(
        store.get(&run.id).unwrap().unwrap().gates.is_empty(),
        "stale gate must not be recorded"
    );
}
