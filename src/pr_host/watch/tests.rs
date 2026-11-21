//! The shared watch loop, with an adapter that answers from a list of snapshots and a clock that never
//! sleeps. Reading the snapshots from `gh` and `az` is tested in the shared adapter suite.

use super::*;
use crate::{
    pr_host::{Head, ToolError, ToolOutput},
    pr_wait::{Failure, render},
};
use std::{cell::RefCell, collections::VecDeque};

const PR: &str = "https://github.com/o/r/pull/7";

/// Answers each snapshot with the next one of a list; the last one repeats.
struct Snapshots(RefCell<VecDeque<Snapshot>>);

impl Snapshots {
    fn new(list: Vec<Snapshot>) -> Self {
        Self(RefCell::new(list.into()))
    }
}

impl Adapter for Snapshots {
    fn state(&self, _: &dyn Tools, _: &str) -> Result<PrState, ToolProblem> {
        panic!("the watch reads snapshots only")
    }

    fn head(&self, _: &dyn Tools, _: &str) -> Result<Head, ToolProblem> {
        panic!("the watch reads snapshots only")
    }

    fn find_open(
        &self,
        _: &dyn Tools,
        _: &std::path::Path,
        _: &crate::pr_host::Origin,
        _: &str,
    ) -> Result<Option<crate::pr_host::OpenPr>, ToolProblem> {
        panic!("the watch reads snapshots only")
    }

    fn create(
        &self,
        _: &dyn Tools,
        _: &std::path::Path,
        _: &crate::pr_host::Origin,
        _: &crate::pr_host::NewPr,
    ) -> Result<String, ToolProblem> {
        panic!("the watch reads snapshots only")
    }

    fn snapshot(&self, _: &dyn Tools, _: &str) -> Result<Snapshot, ToolProblem> {
        let mut list = self.0.borrow_mut();
        Ok(if list.len() > 1 {
            list.pop_front().unwrap()
        } else {
            list.front().expect("the script has a snapshot").clone()
        })
    }
}

/// No tool is started: the snapshots come from the list.
struct NoTools;

impl Tools for NoTools {
    fn gh(&self, _: Option<&std::path::Path>, _: &[&str]) -> Result<ToolOutput, ToolError> {
        panic!("the watch starts no tool itself")
    }

    fn az(&self, _: &[&str]) -> Result<ToolOutput, ToolError> {
        panic!("the watch starts no tool itself")
    }
}

#[derive(Default)]
struct FakeClock {
    now: RefCell<Duration>,
    sleeps: RefCell<Vec<Duration>>,
}

impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        *self.now.borrow()
    }
    fn sleep(&self, duration: Duration) {
        *self.now.borrow_mut() += duration;
        self.sleeps.borrow_mut().push(duration);
    }
}

/// An open pull request at `head`, up to date with its base, with `checks` as `(name, bucket)`.
fn open(head: &str, checks: &[(&str, &str)]) -> Snapshot {
    Snapshot {
        state: PrState::Open,
        head: head.into(),
        checks: checks
            .iter()
            .map(|(name, bucket)| CheckResult {
                name: (*name).into(),
                bucket: (*bucket).into(),
                link: format!("https://example.com/{name}"),
            })
            .collect(),
        applies: !checks.is_empty(),
        base: Some(BaseState::UpToDate),
    }
}

fn in_state(state: PrState, head: &str) -> Snapshot {
    Snapshot {
        state,
        checks: Vec::new(),
        applies: false,
        ..open(head, &[])
    }
}

struct Watched {
    waited: Waited,
    activity: Vec<String>,
    clock: FakeClock,
}

fn run(snapshots: Vec<Snapshot>, options: &Options) -> Watched {
    let clock = FakeClock::default();
    let mut activity = Vec::new();
    let pr = PullRequest {
        adapter: &Snapshots::new(snapshots),
        tools: &NoTools,
        url: PR,
    };
    let waited = watch(
        &pr,
        options,
        &clock,
        &mut |text| activity.push(text.to_string()),
        &mut |_| {},
    )
    .unwrap();
    Watched {
        waited,
        activity,
        clock,
    }
}

fn sleeps(watched: &Watched) -> Vec<Duration> {
    watched.clock.sleeps.borrow().clone()
}

#[test]
fn green_checks_give_passed_and_the_activity_follows_the_wait() {
    let checks = [("build", "pass"), ("lint", "skipping")];
    let watched = run(vec![open("aaa111", &checks)], &Options::default());
    let waited = &watched.waited;
    assert_eq!((waited.outcome, waited.exit_code), (Outcome::Passed, 0));
    assert_eq!(waited.base, BaseState::UpToDate);
    assert_eq!(waited.head, "aaa111");
    assert_eq!(waited.state, PrState::Open);
    assert_eq!(waited.checks.len(), 2);
    assert_eq!(watched.activity, ["Waiting for CI", "CI passed"]);
}

#[test]
fn pending_checks_are_waited_for_one_poll_at_a_time() {
    let pending = open("aaa", &[("build", "pending"), ("lint", "pass")]);
    let done = open("aaa", &[("build", "pass"), ("lint", "pass")]);
    let watched = run(
        vec![pending.clone(), pending.clone(), pending, done],
        &Options::default(),
    );
    assert_eq!(watched.waited.outcome, Outcome::Passed);
    assert_eq!(sleeps(&watched), [Duration::from_secs(15); 2]);
}

#[test]
fn a_cancelled_check_without_a_failure_gives_cancelled() {
    let checks = [("build", "cancel"), ("lint", "pass")];
    let watched = run(vec![open("aaa", &checks)], &Options::default());
    assert_eq!(
        (watched.waited.outcome, watched.waited.exit_code),
        (Outcome::Cancelled, 3)
    );
}

#[test]
fn a_failed_check_gives_failed_with_the_failures_listed() {
    let checks = [("build", "fail"), ("lint", "cancel")];
    let watched = run(vec![open("aaa", &checks)], &Options::default());
    let waited = &watched.waited;
    assert_eq!((waited.outcome, waited.exit_code), (Outcome::Failed, 2));
    let failure = Failure {
        name: "build".into(),
        link: "https://example.com/build".into(),
        excerpt: None,
    };
    assert_eq!(waited.failures, [failure]);
    assert_eq!(watched.activity.last().unwrap(), "CI failed: build");
}

#[test]
fn fail_fast_returns_at_the_first_failure_and_lists_what_still_runs() {
    let snapshots = || {
        vec![
            open("aaa", &[]),
            open("aaa", &[("a", "pending"), ("b", "pending")]),
            open("aaa", &[("a", "fail"), ("b", "pending")]),
            open("aaa", &[("a", "fail"), ("b", "pass")]),
        ]
    };
    let fast = Options {
        fail_fast: true,
        ..Options::default()
    };
    let watched = run(snapshots(), &fast);
    assert_eq!(watched.waited.outcome, Outcome::Failed);
    assert_eq!(sleeps(&watched).len(), 1);
    assert!(
        watched
            .waited
            .checks
            .iter()
            .any(|c| c.name == "b" && c.bucket == "pending")
    );

    // Without it the wait goes on until every check has ended.
    let watched = run(snapshots(), &Options::default());
    assert_eq!(watched.waited.outcome, Outcome::Failed);
    assert_eq!(sleeps(&watched).len(), 2);
}

#[test]
fn a_check_that_never_ends_gives_timeout_with_the_pending_checks() {
    let options = Options {
        timeout: Duration::from_secs(300),
        ..Options::default()
    };
    let checks = [("build", "pending"), ("lint", "pass")];
    let watched = run(vec![open("aaa", &checks)], &options);
    let waited = &watched.waited;
    assert_eq!((waited.outcome, waited.exit_code), (Outcome::Timeout, 4));
    assert_eq!(watched.clock.elapsed(), Duration::from_secs(300));
    assert!(
        waited
            .checks
            .iter()
            .any(|c| c.name == "build" && c.bucket == "pending")
    );
    assert_eq!(watched.activity.last().unwrap(), "CI timed out");
    assert!(render(waited).contains("build"));
}

#[test]
fn without_checks_it_waits_the_grace_period_before_no_ci_and_takes_late_ones() {
    let watched = run(vec![open("aaa", &[])], &Options::default());
    assert_eq!(
        (watched.waited.outcome, watched.waited.exit_code),
        (Outcome::NoCi, 6)
    );
    assert_eq!(watched.clock.elapsed(), Duration::from_secs(90));

    // A check that applies ends the grace; one that does not leaves it running.
    let mut skipped = open("aaa", &[("optional", "skipping")]);
    skipped.applies = false;
    let late = open("aaa", &[("build", "fail")]);
    let watched = run(vec![open("aaa", &[]), skipped, late], &Options::default());
    assert_eq!(watched.waited.outcome, Outcome::Failed);
    assert!(watched.clock.elapsed() < Duration::from_secs(90));
}

#[test]
fn a_new_head_while_waiting_ends_the_wait_without_check_results() {
    let watched = run(
        vec![open("aaa", &[]), open("bbb", &[("build", "pending")])],
        &Options::default(),
    );
    let waited = &watched.waited;
    assert_eq!(
        (waited.outcome, waited.exit_code),
        (Outcome::HeadChanged, 5)
    );
    assert_eq!(waited.head, "aaa");
    assert_eq!(waited.new_head.as_deref(), Some("bbb"));
    assert!(waited.checks.is_empty());
}

#[test]
fn a_merged_or_closed_pull_request_is_not_watched_and_its_state_is_reported() {
    for state in [PrState::Merged, PrState::Closed] {
        let watched = run(vec![in_state(state, "aaa")], &Options::default());
        let waited = &watched.waited;
        assert_eq!((waited.outcome, waited.exit_code), (Outcome::NotOpen, 7));
        assert_eq!(waited.state, state);
        assert_eq!(waited.head, "aaa");
        assert!(sleeps(&watched).is_empty());
        assert_eq!(watched.activity, ["Pull request is not open"]);
    }

    // A pull request that merges while its checks run ends the wait the same way.
    let snapshots = vec![
        open("aaa", &[]),
        open("aaa", &[("build", "pending")]),
        in_state(PrState::Merged, "aaa"),
    ];
    let waited = run(snapshots, &Options::default()).waited;
    assert_eq!(
        (waited.outcome, waited.state),
        (Outcome::NotOpen, PrState::Merged)
    );
    assert!(waited.checks.is_empty());
}

#[test]
fn a_draft_is_watched_unless_only_a_ready_pull_request_counts() {
    let snapshots = || {
        vec![
            in_state(PrState::Draft, "aaa"),
            open("aaa", &[("build", "pass")]),
        ]
    };
    let watched = run(snapshots(), &Options::default());
    assert_eq!(
        (watched.waited.outcome, watched.waited.state),
        (Outcome::Passed, PrState::Open)
    );

    let ready_only = Options {
        ready_only: true,
        ..Options::default()
    };
    let watched = run(snapshots(), &ready_only);
    let waited = &watched.waited;
    assert_eq!(
        (waited.outcome, waited.state),
        (Outcome::NotOpen, PrState::Draft)
    );
    assert!(sleeps(&watched).is_empty());
}

#[test]
fn a_just_pushed_commit_is_waited_for_before_its_checks_are_watched() {
    // Right after a push the host still reports the old head; it shows the pushed one on the next ask.
    let options = Options {
        pushed_head: Some("new222".into()),
        ..Options::default()
    };
    let snapshots = vec![
        open("old111", &[]),
        open("new222", &[]),
        open("new222", &[("build", "pass")]),
    ];
    let watched = run(snapshots, &options);
    let waited = &watched.waited;
    assert_eq!(waited.outcome, Outcome::Passed);
    assert_eq!(waited.head, "new222");
    assert_eq!(waited.new_head, None);
    assert_eq!(sleeps(&watched), [Duration::from_secs(5)]);
}

#[test]
fn a_base_state_the_host_has_not_worked_out_is_asked_for_again() {
    let mut unknown = open("aaa", &[("build", "pass")]);
    unknown.base = None;
    let mut behind = unknown.clone();
    behind.base = Some(BaseState::Behind);
    let watched = run(
        vec![unknown.clone(), unknown.clone(), behind],
        &Options::default(),
    );
    assert_eq!(watched.waited.base, BaseState::Behind);
    assert_eq!(sleeps(&watched), [Duration::from_secs(3)]);

    // It stays unknown after a few tries.
    let watched = run(vec![unknown], &Options::default());
    assert_eq!(watched.waited.base, BaseState::Unknown);
    assert_eq!(sleeps(&watched), [Duration::from_secs(3); 5]);
}
