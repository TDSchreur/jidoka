//! `pr wait` against a fake pull request host and a fake server.

use super::*;
use crate::pr_host::{FakeHost, Head};
use crate::pr_state::{PrStateRecorder, PrStatus};
use crate::run::{Run, RunStatus, RunStore};
use crate::test_support::{TestDir, new_run};
use std::cell::RefCell;

const PR: &str = "https://github.com/o/r/pull/7";

/// Keeps what `pr wait` records; `refuse` makes every record fail with that message.
#[derive(Default)]
struct Recorder {
    activities: RefCell<Vec<String>>,
    states: RefCell<Vec<PrStatus>>,
    refuse: Option<String>,
}

impl PrStateRecorder for Recorder {
    fn record_pr_state(&self, _id: &str, status: &PrStatus) -> Result<bool, String> {
        self.states.borrow_mut().push(status.clone());
        self.refuse.clone().map_or(Ok(true), Err)
    }
}

impl WaitRecorder for Recorder {
    fn activity(&self, _id: &str, text: &str) -> Result<(), String> {
        self.activities.borrow_mut().push(text.to_owned());
        self.refuse.clone().map_or(Ok(()), Err)
    }
}

struct NoClock;

impl Clock for NoClock {
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn sleep(&self, _duration: Duration) {}
}

/// A stored run with pull request `pr` and `status`.
fn run(pr: Option<&str>, status: RunStatus) -> (TestDir, Run) {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let mut run = new_run(&store, "task");
    run.pr = pr.map(str::to_owned);
    run.status = status;
    (dir, run)
}

fn host(state: PrState) -> FakeHost {
    FakeHost::new(vec![Ok(Head {
        state,
        commit: "abc1234".into(),
        branch: "feature".into(),
    })])
}

/// Waits for `run` and returns the outcome and the progress lines.
fn wait_for(
    run: &Run,
    host: &FakeHost,
    recorder: &Recorder,
) -> (Result<Waited, String>, Vec<String>) {
    let mut progress = Vec::new();
    let waited = wait(
        run,
        &Options::default(),
        host,
        recorder,
        &NoClock,
        &mut |text| progress.push(text.to_owned()),
    );
    (waited, progress)
}

#[test]
fn a_running_run_follows_the_wait_in_its_activity() {
    let (_dir, run) = run(Some(PR), RunStatus::Running);
    let recorder = Recorder::default();
    let (waited, progress) = wait_for(&run, &host(PrState::Open), &recorder);
    assert_eq!(waited.unwrap().outcome, Outcome::Passed);
    assert_eq!(recorder.activities.into_inner(), ["CI passed"]);
    assert!(recorder.states.into_inner().is_empty());
    assert!(progress.is_empty(), "{progress:?}");
}

#[test]
fn a_run_that_is_not_running_keeps_its_activity() {
    let (_dir, run) = run(Some(PR), RunStatus::Blocked);
    let recorder = Recorder::default();
    let (waited, _) = wait_for(&run, &host(PrState::Open), &recorder);
    assert_eq!(waited.unwrap().outcome, Outcome::Passed);
    assert!(recorder.activities.into_inner().is_empty());
}

#[test]
fn a_pull_request_that_is_not_open_is_recorded_so_the_run_follows_it() {
    let (_dir, run) = run(Some(PR), RunStatus::Running);
    let recorder = Recorder::default();
    let (waited, progress) = wait_for(&run, &host(PrState::Merged), &recorder);
    assert_eq!(waited.unwrap().outcome, Outcome::NotOpen);
    let states = recorder.states.into_inner();
    assert_eq!(states.len(), 1);
    assert_eq!(
        (states[0].url.as_str(), states[0].state),
        (PR, PrState::Merged)
    );
    assert_eq!(
        progress,
        [crate::pr_state::describe(PrState::Merged, true).trim_end()]
    );
}

#[test]
fn problems_recording_are_shown_and_do_not_end_the_wait() {
    let (_dir, run) = run(Some(PR), RunStatus::Running);
    let recorder = Recorder {
        refuse: Some("no dashboard".into()),
        ..Recorder::default()
    };
    let (waited, progress) = wait_for(&run, &host(PrState::Closed), &recorder);
    assert_eq!(waited.unwrap().outcome, Outcome::NotOpen);
    assert_eq!(
        progress,
        [
            "The run's activity could not be updated: no dashboard",
            "The run could not be updated: no dashboard",
        ]
    );
}

#[test]
fn a_run_without_a_pull_request_is_refused() {
    let (_dir, run) = run(None, RunStatus::Running);
    let (waited, _) = wait_for(&run, &host(PrState::Open), &Recorder::default());
    assert!(waited.unwrap_err().contains("no pull request recorded"));
}
