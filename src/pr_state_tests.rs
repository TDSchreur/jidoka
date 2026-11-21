//! The refresh pass and the Pull request state rule against a real run store and a fake pull request host.

use super::*;
use crate::pr_host::{FakeHost, Head};
use crate::run::{NewRun, RunStatus, RunStore, Stage, UpdateRun};
use crate::test_support::{TestDir, advance_to};

const PR: &str = "https://github.com/o/r/pull/7";

fn store() -> (TestDir, RunStore) {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    (dir, store)
}

fn run_with_pr(store: &RunStore, pr: &str) -> String {
    run_with_pr_in(store, pr, Stage::Finish)
}

fn run_with_pr_in(store: &RunStore, pr: &str, stage: Stage) -> String {
    let run = store
        .create(NewRun {
            title: "t".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        })
        .unwrap();
    let update: UpdateRun = serde_json::from_str(&format!(r#"{{"pr":"{pr}"}}"#)).unwrap();
    advance_to(store, &run.id, stage);
    store.update(&run.id, update).unwrap();
    run.id
}

fn stored(store: &RunStore, id: &str) -> Option<PrStatus> {
    store
        .list()
        .unwrap()
        .into_iter()
        .find(|run| run.id == id)
        .unwrap()
        .pr_status
}

#[test]
fn merged_is_final_and_others_are_checked_again_after_a_minute() {
    let (_dir, store) = store();
    run_with_pr(&store, PR);
    let host = FakeHost::states(&[PrState::Open, PrState::Merged]);
    refresh(&store, &host, 0).unwrap();
    // Within the minute nothing is asked again.
    assert_eq!(refresh(&store, &host, 59_999).unwrap(), 0);
    // After it, the open one is, and it has been merged.
    assert_eq!(refresh(&store, &host, 60_000).unwrap(), 1);
    // Merged is never asked again, however much later.
    assert_eq!(refresh(&store, &host, 10_000_000).unwrap(), 0);
    assert_eq!(host.calls(), [format!("state {PR}"), format!("state {PR}")]);
}

#[test]
fn a_problem_is_stored_as_unknown_with_its_reason_and_tried_again() {
    let (_dir, store) = store();
    let id = run_with_pr(&store, PR);
    let closed = Head {
        state: PrState::Closed,
        commit: "abc1234".into(),
        branch: "feature".into(),
    };
    let host = FakeHost::new(vec![Err("The tool is not installed.".into()), Ok(closed)]);
    refresh(&store, &host, 0).unwrap();
    let unknown = stored(&store, &id).unwrap();
    assert_eq!(unknown.state, PrState::Unknown);
    assert_eq!(
        unknown.reason.as_deref(),
        Some("The tool is not installed.")
    );
    refresh(&store, &host, 60_000).unwrap();
    assert_eq!(stored(&store, &id).unwrap().state, PrState::Closed);
}

#[test]
fn a_new_pull_request_url_is_checked_at_once_and_runs_without_one_are_left_alone() {
    let (_dir, store) = store();
    let id = run_with_pr(&store, PR);
    store
        .create(NewRun {
            title: "no pr".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        })
        .unwrap();
    let host = FakeHost::states(&[PrState::Merged, PrState::Draft]);
    assert_eq!(refresh(&store, &host, 0).unwrap(), 1);
    // The Run now records another pull request: its stored state is for the old URL, so it is read again.
    let other = "https://github.com/o/r/pull/8";
    let update: UpdateRun = serde_json::from_str(&format!(r#"{{"pr":"{other}"}}"#)).unwrap();
    store.update(&id, update).unwrap();
    assert_eq!(refresh(&store, &host, 1).unwrap(), 1);
    let status = stored(&store, &id).unwrap();
    assert_eq!((status.url.as_str(), status.state), (other, PrState::Draft));
}

fn run_waiting_for_ci(store: &RunStore) -> String {
    let id = run_with_pr(store, PR);
    let update: UpdateRun = serde_json::from_str(r#"{"activity":"Waiting for CI"}"#).unwrap();
    store.update(&id, update).unwrap();
    id
}

fn the_run(store: &RunStore, id: &str) -> crate::run::Run {
    store
        .list()
        .unwrap()
        .into_iter()
        .find(|run| run.id == id)
        .unwrap()
}

#[test]
fn a_merged_pull_request_finishes_the_run_and_clears_its_activity() {
    let (_dir, store) = store();
    let id = run_waiting_for_ci(&store);
    refresh(&store, &FakeHost::states(&[PrState::Merged]), 0).unwrap();
    let run = the_run(&store, &id);
    assert_eq!((run.stage, run.status), (Stage::Finish, RunStatus::Done));
    assert_eq!(run.activity, None);
    assert_eq!(run.summaries[&Stage::Finish], "Pull request merged");
    assert_eq!(
        run.completion,
        Some(crate::run::Completion::Merged { url: PR.into() })
    );
}

#[test]
fn syncing_one_run_reads_the_host_now_even_when_the_state_is_recent() {
    let (_dir, store) = store();
    let id = run_waiting_for_ci(&store);
    let host = FakeHost::states(&[PrState::Open, PrState::Merged]);
    refresh(&store, &host, 0).unwrap();
    let (status, _) = sync_run(&store, &host, &id, 1).unwrap().unwrap();
    assert_eq!(status.state, PrState::Merged);
    assert_eq!(stored(&store, &id).unwrap().checked_at, 1);
    assert_eq!(stored(&store, &id).unwrap().state, PrState::Merged);
    assert_eq!(the_run(&store, &id).status, RunStatus::Done);
    assert_eq!(host.calls().len(), 2);
}

/// Replaces the PR while the host request is in flight.
struct ReplacingHost<'a> {
    store: &'a RunStore,
    id: &'a str,
    state: PrState,
}

impl PrHost for ReplacingHost<'_> {
    fn state(&self, _url: &str) -> Result<PrState, ToolProblem> {
        let update: UpdateRun =
            serde_json::from_str(r#"{"pr":"https://github.com/o/r/pull/8"}"#).unwrap();
        self.store.update(self.id, update).unwrap();
        Ok(self.state)
    }

    fn head(&self, _url: &str) -> Result<Head, ToolProblem> {
        panic!("the refresh reads only the state")
    }

    fn find_open(
        &self,
        _dir: &std::path::Path,
        _origin: &crate::pr_host::Origin,
        _branch: &str,
    ) -> Result<Option<crate::pr_host::OpenPr>, ToolProblem> {
        panic!("the refresh reads only the state")
    }

    fn check_new(
        &self,
        _origin: &crate::pr_host::Origin,
        _pr: &crate::pr_host::NewPr,
    ) -> Result<(), ToolProblem> {
        panic!("the refresh reads only the state")
    }

    fn create(
        &self,
        _dir: &std::path::Path,
        _origin: &crate::pr_host::Origin,
        _pr: &crate::pr_host::NewPr,
    ) -> Result<String, ToolProblem> {
        panic!("the refresh reads only the state")
    }

    fn watch(
        &self,
        _url: &str,
        _options: &crate::pr_wait::Options,
        _clock: &dyn crate::pr_wait::Clock,
        _activity: &mut dyn FnMut(&str),
        _progress: &mut dyn FnMut(&str),
    ) -> Result<crate::pr_wait::Waited, ToolProblem> {
        panic!("the refresh reads only the state")
    }
}

#[test]
fn superseded_host_results_do_not_change_the_run_or_its_cached_status() {
    for state in [PrState::Merged, PrState::Closed] {
        let (_dir, store) = store();
        let id = run_with_pr(&store, PR);
        let host = ReplacingHost {
            store: &store,
            id: &id,
            state,
        };
        refresh(&store, &host, 123).unwrap();
        let run = the_run(&store, &id);
        assert_eq!(run.pr.as_deref(), Some("https://github.com/o/r/pull/8"));
        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.pr_status, None);
    }
}

#[test]
fn one_rule_decides_which_pull_request_state_wins() {
    let status = |state, checked_at| PrStatus {
        url: PR.into(),
        state,
        reason: None,
        checked_at,
    };
    let merged = status(PrState::Merged, 20);
    assert!(!status(PrState::Open, 21).supersedes(&merged));
    assert!(status(PrState::Merged, 21).supersedes(&merged));
    assert!(!status(PrState::Closed, 19).supersedes(&status(PrState::Open, 20)));
    assert!(status(PrState::Open, 21).supersedes(&status(PrState::Closed, 20)));
    assert!(status(PrState::Closed, 20).supersedes(&status(PrState::Open, 20)));
    assert!(!status(PrState::Open, 20).supersedes(&status(PrState::Closed, 20)));
    assert!(!status(PrState::Draft, 20).supersedes(&status(PrState::Open, 20)));
    let other = PrStatus {
        url: "https://github.com/o/r/pull/8".into(),
        ..status(PrState::Open, 1)
    };
    assert!(other.supersedes(&merged));
}

#[test]
fn older_and_tied_results_cannot_overwrite_newer_or_terminal_results() {
    use crate::run::PrResult;
    let (_dir, store) = store();
    let id = run_with_pr(&store, PR);
    let status = |state, checked_at| PrStatus {
        url: PR.into(),
        state,
        reason: None,
        checked_at,
    };
    assert_eq!(
        store
            .record_pr_result(&id, status(PrState::Open, 20))
            .unwrap(),
        PrResult::Accepted { changed: false }
    );
    assert_eq!(
        store
            .record_pr_result(&id, status(PrState::Closed, 19))
            .unwrap(),
        PrResult::Discarded
    );
    assert_eq!(
        store
            .record_pr_result(&id, status(PrState::Closed, 20))
            .unwrap(),
        PrResult::Accepted { changed: true }
    );
    assert_eq!(
        store
            .record_pr_result(&id, status(PrState::Open, 20))
            .unwrap(),
        PrResult::Discarded
    );
    assert_eq!(
        store
            .record_pr_result(&id, status(PrState::Merged, 20))
            .unwrap(),
        PrResult::Accepted { changed: true }
    );
    assert_eq!(
        store
            .record_pr_result(&id, status(PrState::Open, 21))
            .unwrap(),
        PrResult::Discarded
    );
    assert_eq!(stored(&store, &id).unwrap().state, PrState::Merged);
    assert_eq!(
        store
            .record_pr_result("999999999", status(PrState::Open, 22))
            .unwrap(),
        PrResult::Missing
    );
}

#[test]
fn repeated_merged_observation_reconciles_after_entering_finish() {
    use crate::run::PrResult;
    let (_dir, store) = store();
    let id = run_with_pr_in(&store, PR, Stage::Build);
    let status = PrStatus {
        url: PR.into(),
        state: PrState::Merged,
        reason: None,
        checked_at: 20,
    };
    assert_eq!(
        store.record_pr_result(&id, status.clone()).unwrap(),
        PrResult::Accepted { changed: false }
    );
    advance_to(&store, &id, Stage::Finish);
    assert_eq!(
        store.record_pr_result(&id, status.clone()).unwrap(),
        PrResult::Accepted { changed: true }
    );
    assert_eq!(
        store.record_pr_result(&id, status).unwrap(),
        PrResult::Accepted { changed: false }
    );
    assert_eq!(the_run(&store, &id).status, RunStatus::Done);
}
