//! `factory pr wait` against a scripted `gh` and a clock that never sleeps.

use super::*;
use serde_json::json;
use std::{cell::RefCell, collections::VecDeque};

const PR: &str = "https://github.com/o/r/pull/7";

/// Answers `gh` calls from queues; the last answer of a queue repeats.
#[derive(Default)]
struct FakeGh {
    views: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    checks: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    logs: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    calls: RefCell<Vec<String>>,
}

fn ok(stdout: impl Into<String>) -> Result<GhOutput, GhError> {
    Ok(GhOutput {
        code: Some(0),
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn failed(code: i32, stderr: &str) -> Result<GhOutput, GhError> {
    Ok(GhOutput {
        code: Some(code),
        stdout: String::new(),
        stderr: stderr.into(),
    })
}

fn view(head: &str, state: &str, merge: &str) -> Result<GhOutput, GhError> {
    ok(json!({"headRefOid": head, "state": state, "mergeStateStatus": merge, "baseRefName": "develop"}).to_string())
}

fn checks(list: &[(&str, &str)]) -> Result<GhOutput, GhError> {
    let items: Vec<_> = list
        .iter()
        .map(|(name, bucket)| {
            json!({"name": name, "bucket": bucket, "state": "X", "link": format!("https://github.com/o/r/actions/runs/{}/job/1", name.len() + 100)})
        })
        .collect();
    ok(serde_json::to_string(&items).unwrap())
}

fn next(queue: &RefCell<VecDeque<Result<GhOutput, GhError>>>) -> Result<GhOutput, GhError> {
    let mut queue = queue.borrow_mut();
    let answer = if queue.len() > 1 {
        queue.pop_front().unwrap()
    } else {
        match queue.front().expect("the script has an answer") {
            Ok(output) => Ok(output.clone()),
            Err(GhError::NotInstalled) => Err(GhError::NotInstalled),
            Err(GhError::Other(text)) => Err(GhError::Other(text.clone())),
        }
    };
    answer
}

impl Gh for FakeGh {
    fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        self.calls.borrow_mut().push(args.join(" "));
        match args {
            ["pr", "view", ..] => next(&self.views),
            ["pr", "checks", ..] => next(&self.checks),
            ["run", "view", ..] if self.logs.borrow().is_empty() => failed(1, "no log"),
            ["run", "view", ..] => next(&self.logs),
            other => panic!("unexpected gh call {other:?}"),
        }
    }
}

impl FakeGh {
    fn script(
        views: Vec<Result<GhOutput, GhError>>,
        checks: Vec<Result<GhOutput, GhError>>,
    ) -> Self {
        Self {
            views: RefCell::new(views.into()),
            checks: RefCell::new(checks.into()),
            ..Self::default()
        }
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

fn run_wait(gh: &FakeGh) -> (Result<Waited, ToolProblem>, Vec<String>, FakeClock) {
    run_wait_with(gh, &Options::default())
}

fn run_wait_with(
    gh: &FakeGh,
    options: &Options,
) -> (Result<Waited, ToolProblem>, Vec<String>, FakeClock) {
    let clock = FakeClock::default();
    let mut activity = Vec::new();
    let result = wait(
        Some(PR),
        options,
        gh,
        &clock,
        &mut |text| activity.push(text.to_string()),
        &mut |_| {},
    );
    (result, activity, clock)
}

#[test]
fn green_checks_give_passed_and_the_activity_follows_the_wait() {
    let gh = FakeGh::script(
        vec![view("aaa111", "OPEN", "CLEAN")],
        vec![checks(&[("build", "pass"), ("lint", "skipping")])],
    );
    let (result, activity, _) = run_wait(&gh);
    let waited = result.unwrap();
    assert_eq!(waited.outcome, Outcome::Passed);
    assert_eq!(waited.outcome.exit_code(), 0);
    assert_eq!(waited.exit_code, 0);
    assert_eq!(waited.base, BaseState::UpToDate);
    assert_eq!(waited.head, "aaa111");
    assert_eq!(waited.checks.len(), 2);
    assert_eq!(activity, ["Waiting for CI", "CI passed"]);
}

#[test]
fn pending_checks_are_waited_for_one_poll_at_a_time() {
    let gh = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN")],
        vec![
            checks(&[("build", "pending"), ("lint", "pass")]),
            checks(&[("build", "pending"), ("lint", "pass")]),
            checks(&[("build", "pass"), ("lint", "pass")]),
        ],
    );
    let (result, _, clock) = run_wait(&gh);
    assert_eq!(result.unwrap().outcome, Outcome::Passed);
    assert_eq!(clock.sleeps.borrow().len(), 2);
    assert!(clock
        .sleeps
        .borrow()
        .iter()
        .all(|d| *d == Duration::from_secs(15)));
}

#[test]
fn a_failure_beats_a_cancel_and_a_cancel_beats_green() {
    let failing = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN")],
        vec![checks(&[("a", "cancel"), ("b", "fail"), ("c", "pass")])],
    );
    let waited = run_wait(&failing).0.unwrap();
    assert_eq!((waited.outcome, waited.exit_code), (Outcome::Failed, 2));

    let cancelled = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN")],
        vec![checks(&[("a", "cancel"), ("c", "pass")])],
    );
    let waited = run_wait(&cancelled).0.unwrap();
    assert_eq!((waited.outcome, waited.exit_code), (Outcome::Cancelled, 3));
}

#[test]
fn a_new_head_while_waiting_ends_the_wait_without_check_results() {
    let gh = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN"), view("bbb", "OPEN", "CLEAN")],
        vec![checks(&[("build", "pending")])],
    );
    let waited = run_wait(&gh).0.unwrap();
    assert_eq!(
        (waited.outcome, waited.exit_code),
        (Outcome::HeadChanged, 5)
    );
    assert_eq!(waited.head, "aaa");
    assert_eq!(waited.new_head.as_deref(), Some("bbb"));
    assert!(waited.checks.is_empty());
}

#[test]
fn a_head_that_changes_just_before_the_end_is_not_reported_as_green() {
    // The checks finish, but the head moved: the result belongs to the old commit.
    let gh = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN"), view("bbb", "OPEN", "CLEAN")],
        vec![checks(&[("build", "pass")])],
    );
    let waited = run_wait(&gh).0.unwrap();
    assert_eq!(waited.outcome, Outcome::HeadChanged);
}

#[test]
fn a_merged_or_closed_pull_request_is_not_watched() {
    for state in ["MERGED", "CLOSED"] {
        let gh = FakeGh::script(
            vec![view("aaa", state, "UNKNOWN")],
            vec![checks(&[("a", "pass")])],
        );
        let waited = run_wait(&gh).0.unwrap();
        assert_eq!(
            (waited.outcome, waited.exit_code),
            (Outcome::NotOpen, 7),
            "{state}"
        );
        assert!(gh
            .calls
            .borrow()
            .iter()
            .all(|c| !c.starts_with("pr checks")));
    }
}

#[test]
fn the_base_state_is_reported_and_unknown_is_asked_again() {
    for (merge, expected) in [
        ("BEHIND", BaseState::Behind),
        ("DIRTY", BaseState::Conflicts),
        ("BLOCKED", BaseState::UpToDate),
        ("UNSTABLE", BaseState::UpToDate),
    ] {
        let gh = FakeGh::script(
            vec![view("aaa", "OPEN", merge)],
            vec![checks(&[("a", "pass")])],
        );
        assert_eq!(run_wait(&gh).0.unwrap().base, expected, "{merge}");
    }
    // UNKNOWN twice, then settled.
    let gh = FakeGh::script(
        vec![
            view("aaa", "OPEN", "CLEAN"),
            view("aaa", "OPEN", "UNKNOWN"),
            view("aaa", "OPEN", "UNKNOWN"),
            view("aaa", "OPEN", "BEHIND"),
        ],
        vec![checks(&[("a", "pass")])],
    );
    let (result, _, clock) = run_wait(&gh);
    assert_eq!(result.unwrap().base, BaseState::Behind);
    assert!(clock
        .sleeps
        .borrow()
        .iter()
        .any(|d| *d == Duration::from_secs(3)));
    // UNKNOWN for good gives unknown after five more asks.
    let gh = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN"), view("aaa", "OPEN", "UNKNOWN")],
        vec![checks(&[("a", "pass")])],
    );
    let (result, _, _) = run_wait(&gh);
    assert_eq!(result.unwrap().base, BaseState::Unknown);
    let asks = gh
        .calls
        .borrow()
        .iter()
        .filter(|c| c.starts_with("pr view"))
        .count();
    assert!((6..=8).contains(&asks), "{asks}");
}

#[test]
fn tool_problems_say_what_to_do() {
    let problem = |gh: FakeGh| run_wait(&gh).0.unwrap_err().0;

    let missing = FakeGh::script(vec![Err(GhError::NotInstalled)], vec![checks(&[])]);
    assert!(problem(missing).contains("GitHub CLI"));

    let signed_out = FakeGh::script(
        vec![failed(
            4,
            "To get started with GitHub CLI, please run:  gh auth login",
        )],
        vec![checks(&[])],
    );
    assert!(problem(signed_out).contains("gh auth login"));

    let offline = FakeGh::script(
        vec![failed(
            1,
            "error connecting to api.github.com\ncheck your internet connection",
        )],
        vec![checks(&[])],
    );
    assert!(problem(offline).contains("network"));

    let unknown = FakeGh::script(
        vec![failed(1, "GraphQL: Could not resolve to a PullRequest")],
        vec![checks(&[])],
    );
    assert!(problem(unknown).contains("Could not resolve"));

    let none = FakeGh::script(vec![view("a", "OPEN", "CLEAN")], vec![checks(&[])]);
    let clock = FakeClock::default();
    let error = wait(
        None,
        &Options::default(),
        &none,
        &clock,
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        error.0.contains("--pr") || error.0.contains("pull request"),
        "{}",
        error.0
    );
}

#[test]
fn the_text_and_the_json_say_the_same_thing() {
    let gh = FakeGh::script(
        vec![view("aaa111", "OPEN", "BEHIND")],
        vec![checks(&[("build", "pass")])],
    );
    let waited = run_wait(&gh).0.unwrap();
    let text = render(&waited);
    assert!(text.starts_with("CI passed"), "{text}");
    assert!(text.contains("behind"), "{text}");
    let json = serde_json::to_value(&waited).unwrap();
    assert_eq!(json["outcome"], "passed");
    assert_eq!(json["exit_code"], 0);
    assert_eq!(json["base"], "behind");
    assert_eq!(json["head"], "aaa111");
    assert_eq!(json["checks"][0]["name"], "build");
}

#[test]
fn progress_says_how_many_checks_still_run() {
    let gh = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN")],
        vec![
            checks(&[("a", "pending"), ("b", "pending"), ("c", "pass")]),
            checks(&[("a", "pass"), ("b", "pending"), ("c", "pass")]),
            checks(&[("a", "pass"), ("b", "pass"), ("c", "pass")]),
        ],
    );
    let clock = FakeClock::default();
    let mut lines = Vec::new();
    wait(
        Some(PR),
        &Options::default(),
        &gh,
        &clock,
        &mut |_| {},
        &mut |text| lines.push(text.to_string()),
    )
    .unwrap();
    assert_eq!(
        lines,
        ["2 of 3 checks still running", "1 of 3 checks still running"]
    );
}

#[test]
fn the_options_are_json_only_and_nothing_repeats() {
    assert!(!parse_options(&[]).unwrap().1);
    assert!(parse_options(&["--json"]).unwrap().1);
    for bad in [&["--json", "--json"][..], &["--bogus"], &["x"]] {
        assert!(parse_options(bad).is_err(), "{bad:?}");
    }
}

// --- time, fail-fast and failure details ------------------------------------------------------------

fn linked(list: &[(&str, &str, &str)]) -> Result<GhOutput, GhError> {
    let items: Vec<_> = list
        .iter()
        .map(|(name, bucket, link)| json!({"name": name, "bucket": bucket, "state": "X", "link": link}))
        .collect();
    ok(serde_json::to_string(&items).unwrap())
}

#[test]
fn checks_that_appear_late_are_waited_for_and_none_at_all_is_no_ci() {
    let late = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN")],
        vec![checks(&[]), checks(&[]), checks(&[("build", "pass")])],
    );
    assert_eq!(run_wait(&late).0.unwrap().outcome, Outcome::Passed);

    let none = FakeGh::script(vec![view("aaa", "OPEN", "BEHIND")], vec![checks(&[])]);
    let (result, activity, clock) = run_wait(&none);
    let waited = result.unwrap();
    assert_eq!((waited.outcome, waited.exit_code), (Outcome::NoCi, 6));
    assert!(
        clock.elapsed() >= Duration::from_secs(90),
        "{:?}",
        clock.elapsed()
    );
    assert!(clock.elapsed() < Duration::from_secs(120));
    assert_eq!(activity.last().unwrap(), "No CI checks");
    // The base state is still reported.
    assert_eq!(waited.base, BaseState::Behind);
}

#[test]
fn a_check_that_never_ends_gives_timeout_with_the_pending_checks() {
    let gh = FakeGh::script(
        vec![view("aaa", "OPEN", "CLEAN")],
        vec![checks(&[("build", "pending"), ("lint", "pass")])],
    );
    let options = Options {
        timeout: Duration::from_secs(300),
        ..Options::default()
    };
    let (result, activity, clock) = run_wait_with(&gh, &options);
    let waited = result.unwrap();
    assert_eq!((waited.outcome, waited.exit_code), (Outcome::Timeout, 4));
    assert!(clock.elapsed() >= Duration::from_secs(300));
    assert!(clock.elapsed() < Duration::from_secs(330));
    assert!(waited
        .checks
        .iter()
        .any(|c| c.name == "build" && c.bucket == "pending"));
    assert_eq!(activity.last().unwrap(), "CI timed out");
    assert!(render(&waited).contains("build"));
}

#[test]
fn fail_fast_returns_at_the_first_failure_and_lists_what_still_runs() {
    let script = || {
        FakeGh::script(
            vec![view("aaa", "OPEN", "CLEAN")],
            vec![
                checks(&[("a", "pending"), ("b", "pending")]),
                checks(&[("a", "fail"), ("b", "pending")]),
                checks(&[("a", "fail"), ("b", "pass")]),
            ],
        )
    };
    let fast = Options {
        fail_fast: true,
        ..Options::default()
    };
    let (result, _, clock) = run_wait_with(&script(), &fast);
    let waited = result.unwrap();
    assert_eq!(waited.outcome, Outcome::Failed);
    assert_eq!(clock.sleeps.borrow().len(), 1);
    assert!(waited
        .checks
        .iter()
        .any(|c| c.name == "b" && c.bucket == "pending"));

    // Without it the wait goes on until every check has ended.
    let (result, _, clock) = run_wait(&script());
    assert_eq!(result.unwrap().outcome, Outcome::Failed);
    assert_eq!(clock.sleeps.borrow().len(), 2);
}

#[test]
fn a_failure_comes_with_the_end_of_its_log() {
    let log: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    let gh = FakeGh {
        logs: RefCell::new(VecDeque::from([ok(log)])),
        ..FakeGh::script(
            vec![view("aaa", "OPEN", "CLEAN")],
            vec![linked(&[(
                "build",
                "fail",
                "https://github.com/o/r/actions/runs/555/job/9",
            )])],
        )
    };
    let waited = run_wait(&gh).0.unwrap();
    let failure = &waited.failures[0];
    assert_eq!(
        (failure.name.as_str(), failure.link.as_str()),
        ("build", "https://github.com/o/r/actions/runs/555/job/9")
    );
    let excerpt = failure.excerpt.as_deref().unwrap();
    assert_eq!(excerpt.lines().count(), 40);
    assert!(excerpt.starts_with("line 61") && excerpt.ends_with("line 100"));
    assert!(
        gh.calls
            .borrow()
            .iter()
            .any(|c| c == "run view 555 --log-failed"),
        "{:?}",
        gh.calls.borrow()
    );
    let text = render(&waited);
    assert!(text.contains("line 100") && text.contains("build"));
}

#[test]
fn log_excerpts_are_cut_by_size_and_fetched_once_per_run_and_for_three_runs_at_most() {
    // One line of 10 KiB: the excerpt keeps the end, within 4 KiB.
    let big = format!("{}END", "x".repeat(10_000));
    let gh = FakeGh {
        logs: RefCell::new(VecDeque::from([ok(big)])),
        ..FakeGh::script(
            vec![view("aaa", "OPEN", "CLEAN")],
            vec![linked(&[
                ("a", "fail", "https://github.com/o/r/actions/runs/1/job/1"),
                ("b", "fail", "https://github.com/o/r/actions/runs/1/job/2"),
                ("c", "fail", "https://github.com/o/r/actions/runs/2/job/3"),
                ("d", "fail", "https://github.com/o/r/actions/runs/3/job/4"),
                ("e", "fail", "https://github.com/o/r/actions/runs/4/job/5"),
            ])],
        )
    };
    let waited = run_wait(&gh).0.unwrap();
    let fetches = gh
        .calls
        .borrow()
        .iter()
        .filter(|c| c.starts_with("run view"))
        .count();
    assert_eq!(
        fetches,
        3,
        "one per run, three runs at most: {:?}",
        gh.calls.borrow()
    );
    let excerpt = waited.failures[0].excerpt.as_deref().unwrap();
    assert!(excerpt.len() <= 4096 && excerpt.ends_with("END"));
    // Every failed check is still listed; the second one of run 1 shares the first one's excerpt.
    assert_eq!(waited.failures.len(), 5);
    assert!(waited.failures[1].excerpt.is_none());
}

#[test]
fn a_log_that_cannot_be_fetched_does_not_fail_the_command() {
    let gh = FakeGh {
        logs: RefCell::new(VecDeque::from([failed(1, "log not found")])),
        ..FakeGh::script(
            vec![view("aaa", "OPEN", "CLEAN")],
            vec![linked(&[
                ("a", "fail", "https://github.com/o/r/actions/runs/1/job/1"),
                ("b", "fail", "not a link"),
            ])],
        )
    };
    let waited = run_wait(&gh).0.unwrap();
    assert_eq!(waited.outcome, Outcome::Failed);
    assert_eq!(waited.failures.len(), 2);
    assert!(waited.failures.iter().all(|f| f.excerpt.is_none()));
}

#[test]
fn the_timeout_option_takes_seconds_minutes_and_hours() {
    let timeout = |text: &str| parse_options(&["--timeout", text]).map(|(o, _)| o.timeout);
    assert_eq!(timeout("90s"), Ok(Duration::from_secs(90)));
    assert_eq!(timeout("45"), Ok(Duration::from_secs(45)));
    assert_eq!(timeout("30m"), Ok(Duration::from_secs(1800)));
    assert_eq!(timeout("2h"), Ok(Duration::from_secs(7200)));
    for bad in [
        "",
        "0",
        "0m",
        "-5",
        "abc",
        "1d",
        "5x",
        "999999999999999999999h",
        "25h",
    ] {
        assert!(timeout(bad).is_err(), "{bad}");
    }
    assert!(parse_options(&["--timeout"]).is_err());
    assert!(parse_options(&["--timeout", "5", "--timeout", "6"]).is_err());
    let (options, json) = parse_options(&["--fail-fast", "--json", "--timeout", "10m"]).unwrap();
    assert!(options.fail_fast && json && options.timeout == Duration::from_secs(600));
    assert!(parse_options(&["--fail-fast", "--fail-fast"]).is_err());
    assert_eq!(Options::default().timeout, Duration::from_secs(1800));
}
