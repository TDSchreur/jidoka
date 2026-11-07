//! `factory pr wait` against a scripted `gh` and a clock that never sleeps.

use super::*;
use serde_json::{json, Value};
use std::{cell::RefCell, collections::VecDeque};

const PR: &str = "https://github.com/o/r/pull/7";

/// Answers `gh` calls from queues; the last answer of a queue repeats.
#[derive(Default)]
struct FakeGh {
    views: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    checks: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    logs: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    calls: RefCell<Vec<String>>,
    azure: RefCell<VecDeque<Result<GhOutput, GhError>>>,
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
    fn az(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        self.calls
            .borrow_mut()
            .push(format!("az {}", args.join(" ")));
        next(&self.azure)
    }
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
            .any(|c| c == "run view 555 --repo o/r --log-failed"),
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

// --- rework 1 ---------------------------------------------------------------------------------------

#[test]
fn a_log_is_asked_for_from_the_repository_of_the_check_link() {
    for (link, repo) in [
        ("https://github.com/o/r/actions/runs/9/job/1", "o/r"),
        (
            "https://ghe.example.com/team/app/actions/runs/9/job/1",
            "ghe.example.com/team/app",
        ),
    ] {
        let gh = FakeGh {
            logs: RefCell::new(VecDeque::from([ok("boom\n")])),
            ..FakeGh::script(
                vec![view("aaa", "OPEN", "CLEAN")],
                vec![linked(&[("build", "fail", link)])],
            )
        };
        run_wait(&gh).0.unwrap();
        let expected = format!("run view 9 --repo {repo} --log-failed");
        assert!(
            gh.calls.borrow().contains(&expected),
            "{:?}",
            gh.calls.borrow()
        );
    }
}

#[test]
fn many_failed_checks_still_fit_the_activity_text() {
    let names: Vec<String> = (0..60)
        .map(|n| format!("job-{n:02}-{}", "x".repeat(30)))
        .collect();
    let list: Vec<(&str, &str, &str)> = names
        .iter()
        .map(|n| {
            (
                n.as_str(),
                "fail",
                "https://github.com/o/r/actions/runs/1/job/1",
            )
        })
        .collect();
    let gh = FakeGh::script(vec![view("aaa", "OPEN", "CLEAN")], vec![linked(&list)]);
    let (result, activity, _) = run_wait(&gh);
    result.unwrap();
    let last = activity.last().unwrap();
    assert!(last.len() <= 500, "{}", last.len());
    assert!(
        last.starts_with("CI failed: job-00") && last.contains("and 57 more"),
        "{last}"
    );
}

#[test]
fn a_timeout_shorter_than_the_grace_period_ends_the_wait_for_checks_too() {
    let gh = FakeGh::script(vec![view("aaa", "OPEN", "CLEAN")], vec![checks(&[])]);
    let options = Options {
        timeout: Duration::from_secs(30),
        ..Options::default()
    };
    let (result, _, clock) = run_wait_with(&gh, &options);
    assert_eq!(result.unwrap().outcome, Outcome::NoCi);
    assert!(
        clock.elapsed() >= Duration::from_secs(30) && clock.elapsed() < Duration::from_secs(60)
    );
}

#[test]
fn names_links_and_log_lines_cannot_write_to_the_terminal() {
    let evil = "x\u{1b}]52;c;AAAA\u{7}y";
    let link = "https://github.com/o/r/actions/runs/1/job/\u{1b}[2J";
    let gh = FakeGh {
        logs: RefCell::new(VecDeque::from([ok(
            "\u{1b}[31mred\u{1b}[0m line\nsecond\u{8}\n",
        )])),
        ..FakeGh::script(
            vec![view("aaa", "OPEN", "CLEAN")],
            vec![linked(&[
                (evil, "fail", link),
                ("ok", "pass", "https://github.com/o/r/actions/runs/1/job/2"),
            ])],
        )
    };
    let (result, activity, _) = run_wait(&gh);
    let waited = result.unwrap();
    let text = render(&waited);
    for output in [text.as_str(), activity.last().unwrap().as_str()] {
        assert!(
            !output.chars().any(|c| c.is_control() && c != '\n'),
            "{output:?}"
        );
    }
    // The visible words are still there.
    assert!(text.contains("red") && text.contains("second"));
    // JSON escapes control characters itself.
    assert!(!serde_json::to_string(&waited).unwrap().contains('\u{1b}'));
}

#[test]
fn the_text_says_nothing_about_the_base_when_the_pull_request_is_not_open() {
    let gh = FakeGh::script(vec![view("aaa", "MERGED", "UNKNOWN")], vec![checks(&[])]);
    let text = render(&run_wait(&gh).0.unwrap());
    assert!(text.starts_with("Pull request is not open"));
    assert!(!text.contains("base"), "{text}");
}

#[test]
fn the_run_and_repository_are_read_from_real_link_shapes() {
    let some = |link: &str| run_of_link(link).map(|(id, repo)| (id.to_string(), repo));
    assert_eq!(
        some("https://github.com/TDSchreur/tds-factory2/actions/runs/42264460135/job/128054364602"),
        Some(("42264460135".into(), "TDSchreur/tds-factory2".into()))
    );
    // A repository whose name starts with a dot, and a link with a query or a fragment.
    assert_eq!(
        some("https://github.com/acme/.github/actions/runs/1/job/2"),
        Some(("1".into(), "acme/.github".into()))
    );
    assert_eq!(
        some("https://github.com/o/r/actions/runs/123?pr=5"),
        Some(("123".into(), "o/r".into()))
    );
    assert_eq!(
        some("https://github.com/o/r/actions/runs/123#step"),
        Some(("123".into(), "o/r".into()))
    );
    // Nothing that could be read as an option, a path or another place.
    for bad in [
        "https://github.com/-x/r/actions/runs/1",
        "https://-host/o/r/actions/runs/1",
        "https://github.com/o/../actions/runs/1",
        "https://github.com/o/./actions/runs/1",
        "https://github.com/a/b/c/actions/runs/1",
        "https://github.com/o/r/actions/runs/",
        "https://github.com/o/r/actions/runs/x1",
        "ftp://github.com/o/r/actions/runs/1",
        "not a link",
    ] {
        assert_eq!(some(bad), None, "{bad}");
    }
}

#[test]
fn azure_abandoned_pr_reports_not_open_without_using_gh() {
    let gh = FakeGh::default();
    gh.azure.borrow_mut().push_back(ok(json!({"pullRequestId":535,"status":"abandoned", "lastMergeSourceCommit":{"commitId":"abc"}, "repository":{"id":"repo","name":"Kahoot","project":{"name":"Projects"}}}).to_string()));
    let waited = wait(
        Some("https://dev.azure.com/dutchworkz/Projects/_git/Kahoot/pullrequest/535"),
        &Options::default(),
        &gh,
        &FakeClock::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(waited.outcome, Outcome::NotOpen);
    assert_eq!(waited.head, "abc");
    assert!(gh.calls.borrow().iter().all(|call| call.starts_with("az ")));
}

struct AzureScript;
impl Gh for AzureScript {
    fn run(&self, _: &[&str]) -> Result<GhOutput, GhError> {
        panic!("GitHub CLI must not be called for Azure")
    }
    fn az(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        match args {
            ["repos", "pr", "show", ..] => ok(json!({"pullRequestId":535,"status":"active", "sourceRefName":"refs/heads/feature", "lastMergeSourceCommit":{"commitId":"abc"}, "repository":{"id":"repo","name":"Kahoot","project":{"name":"Projects"}}}).to_string()),
            ["repos", "ref", "list", ..] => ok(json!([{"name":"refs/heads/feature","objectId":"abc"}]).to_string()),
            ["repos", "pr", "policy", "list", ..] => ok(json!([{"status":"approved", "startedDate":"2026-01-01T00:00:01Z", "configuration":{"isEnabled":true,"isBlocking":true,"type":{"displayName":"Build"}}}]).to_string()),
            ["devops", "invoke", ..] if args.contains(&"pullRequestIterations") => ok(json!({"value":[{"id":1,"createdDate":"2026-01-01T00:00:00Z","sourceRefCommit":{"commitId":"abc"}}]}).to_string()),
            ["devops", "invoke", ..] => ok(json!({"value":[]}).to_string()),
            _ => panic!("unexpected Azure command {args:?}"),
        }
    }
}

#[test]
fn azure_current_approved_required_policy_passes() {
    let waited = wait(
        Some("https://dev.azure.com/dutchworkz/Projects/_git/Kahoot/pullrequest/535"),
        &Options::default(),
        &AzureScript,
        &FakeClock::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(waited.outcome, Outcome::Passed);
    assert_eq!(waited.head, "abc");
    assert_eq!(waited.base, BaseState::Unknown);
}

struct AzureCase {
    policies: Value,
    statuses: Value,
    iteration_statuses: Value,
    iterations: Value,
    /// `None` makes the build lookup fail.
    build: Option<Value>,
    heads: RefCell<VecDeque<&'static str>>,
    pr_states: RefCell<VecDeque<&'static str>>,
    merge_status: Value,
}
impl AzureCase {
    fn new(policies: Value, statuses: Value) -> Self {
        Self {
            policies,
            statuses,
            iteration_statuses: json!({"value":[]}),
            iterations: json!({"value":[{"id":2,"createdDate":"2026-01-01T00:00:00Z","sourceRefCommit":{"commitId":"abc"}}]}),
            build: Some(json!({"status":"completed","result":"canceled"})),
            heads: RefCell::new(VecDeque::from(["abc"])),
            pr_states: RefCell::new(VecDeque::from(["active"])),
            merge_status: Value::Null,
        }
    }
}
impl Gh for AzureCase {
    fn run(&self, _: &[&str]) -> Result<GhOutput, GhError> {
        panic!("unexpected gh call")
    }
    fn az(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        let data = match args {
            ["repos", "pr", "show", ..] => {
                let state = pop_or_repeat(&self.pr_states);
                json!({"pullRequestId":535,"status":state, "mergeStatus":self.merge_status, "sourceRefName":"refs/heads/feature", "lastMergeSourceCommit":{"commitId":"abc"}, "repository":{"id":"repo","name":"Kahoot","project":{"name":"Projects"}}})
            }
            ["repos", "ref", "list", ..] => {
                let head = pop_or_repeat(&self.heads);
                json!([{"name":"refs/heads/feature","objectId":head}])
            }
            ["repos", "pr", "policy", "list", ..] => self.policies.clone(),
            ["pipelines", "runs", "show", ..] => match &self.build {
                Some(build) => build.clone(),
                None => return failed(1, "run not found"),
            },
            ["devops", "invoke", ..] if args.contains(&"pullRequestIterations") => {
                self.iterations.clone()
            }
            ["devops", "invoke", ..] if args.contains(&"pullRequestIterationStatuses") => {
                self.iteration_statuses.clone()
            }
            ["devops", "invoke", ..] if args.contains(&"pullRequestStatuses") => {
                self.statuses.clone()
            }
            _ => panic!("unexpected Azure command {args:?}"),
        };
        ok(data.to_string())
    }
}
fn pop_or_repeat(queue: &RefCell<VecDeque<&'static str>>) -> &'static str {
    let mut queue = queue.borrow_mut();
    if queue.len() > 1 {
        queue.pop_front().unwrap()
    } else {
        queue.front().unwrap()
    }
}
fn azure_policy(state: &str, required: bool) -> Value {
    json!({"status": state, "startedDate":"2026-01-01T00:00:01Z", "configuration":{"isEnabled":true,"isBlocking":required,"type":{"displayName":"Build"}}})
}
fn run_azure(case: &AzureCase, seconds: u64) -> Result<Waited, ToolProblem> {
    run_azure_with(
        case,
        &Options {
            timeout: Duration::from_secs(seconds),
            ..Options::default()
        },
    )
}
fn run_azure_with(gh: &dyn Gh, options: &Options) -> Result<Waited, ToolProblem> {
    wait(
        Some("https://dev.azure.com/dutchworkz/Projects/_git/Kahoot/pullrequest/535"),
        options,
        gh,
        &FakeClock::default(),
        &mut |_| {},
        &mut |_| {},
    )
}

#[test]
fn azure_policy_outcomes_never_treat_pending_or_failure_as_success() {
    for (state, outcome) in [
        ("approved", Outcome::Passed),
        ("rejected", Outcome::Failed),
        ("broken", Outcome::Failed),
        ("queued", Outcome::Timeout),
        ("running", Outcome::Timeout),
        ("unknown", Outcome::Timeout),
    ] {
        let case = AzureCase::new(json!([azure_policy(state, true)]), json!({"value":[]}));
        assert_eq!(run_azure(&case, 1).unwrap().outcome, outcome, "{state}");
    }
}

#[test]
fn azure_optional_policy_does_not_block_but_absent_checks_are_not_ci() {
    let case = AzureCase::new(
        json!([azure_policy("rejected", false)]),
        json!({"value":[]}),
    );
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::NoCi);
}

#[test]
fn azure_unscoped_success_cannot_prove_current_head() {
    let case = AzureCase::new(
        json!([]),
        json!({"value":[{"id":1,"context":{"name":"build"},"state":"succeeded"}]}),
    );
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Timeout);
}

#[test]
fn azure_head_changes_during_poll_are_reported() {
    let case = AzureCase::new(json!([azure_policy("approved", true)]), json!({"value":[]}));
    *case.heads.borrow_mut() = VecDeque::from(["abc", "abc", "def"]);
    let result = run_azure(&case, 1).unwrap();
    assert_eq!(result.outcome, Outcome::HeadChanged);
    assert_eq!(result.new_head.as_deref(), Some("def"));
}

#[test]
fn azure_cancelled_status_is_not_passed() {
    let case = AzureCase::new(
        json!([]),
        json!({"value":[{"id":1,"iterationId":2,"context":{"name":"build"},"state":"cancelled"}]}),
    );
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Cancelled);
}

#[test]
fn azure_cancelled_build_policy_reports_cancelled() {
    let mut policy = azure_policy("rejected", true);
    policy["context"] = json!({"buildId":42});
    let case = AzureCase::new(json!([policy]), json!({"value":[]}));
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Cancelled);
}

#[test]
fn azure_expired_approved_policy_stays_pending() {
    let mut policy = azure_policy("approved", true);
    policy["context"] = json!({"isExpired":true});
    let case = AzureCase::new(json!([policy]), json!({"value":[]}));
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Timeout);
}

#[test]
fn azure_stale_iteration_cannot_pass() {
    let mut case = AzureCase::new(json!([azure_policy("approved", true)]), json!({"value":[]}));
    case.iterations = json!({"value":[{"id":2,"sourceRefCommit":{"commitId":"old"}}]});
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Timeout);
}

#[test]
fn azure_missing_policy_flags_fail_closed() {
    let case = AzureCase::new(
        json!([{"status":"approved", "configuration":{}}]),
        json!({"value":[]}),
    );
    assert!(run_azure(&case, 1).unwrap_err().0.contains("enabled"));
}

#[test]
fn azure_latest_iteration_status_overrides_an_older_failure() {
    let case = AzureCase::new(
        json!([]),
        json!({"value":[{"id":1,"iterationId":2,"context":{"name":"build"},"state":"failed"},{"id":2,"iterationId":2,"context":{"name":"build"},"state":"succeeded"}]}),
    );
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Passed);
}

#[test]
fn azure_unscoped_failure_and_cancellation_do_not_end_current_head_wait() {
    for state in ["failed", "cancelled"] {
        let case = AzureCase::new(
            json!([]),
            json!({"value":[{"id":1,"context":{"name":"build"},"state":state}]}),
        );
        assert_eq!(
            run_azure(&case, 1).unwrap().outcome,
            Outcome::Timeout,
            "{state}"
        );
    }
}

#[test]
fn azure_stale_build_policy_cannot_pass() {
    let mut policy = azure_policy("approved", true);
    policy["startedDate"] = json!("2026-01-01T00:00:00Z");
    let mut case = AzureCase::new(json!([policy]), json!({"value":[]}));
    case.iterations = json!({"value":[{"id":2,"createdDate":"2026-01-02T00:00:00Z","sourceRefCommit":{"commitId":"abc"}}]});
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Timeout);
}

#[test]
fn azure_pr_response_without_project_identity_fails_closed() {
    let gh = FakeGh::default();
    gh.azure.borrow_mut().push_back(ok(json!({"pullRequestId":535,"status":"abandoned","repository":{"id":"repo","name":"Kahoot"}}).to_string()));
    let error = wait(
        Some("https://dev.azure.com/dutchworkz/Projects/_git/Kahoot/pullrequest/535"),
        &Options::default(),
        &gh,
        &FakeClock::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.0.contains("repository"));
}

fn reviewer_policy(state: &str) -> Value {
    json!({"status": state, "startedDate":"2026-01-01T00:00:00Z", "configuration":{"isEnabled":true,"isBlocking":true,"type":{"id":"fa4e907d-c16b-4a4c-9dfa-4906e5d171dd","displayName":"Minimum number of reviewers"}}})
}

#[test]
fn azure_approved_reviewer_policy_from_before_the_latest_push_passes() {
    let mut case = AzureCase::new(json!([reviewer_policy("approved")]), json!({"value":[]}));
    case.iterations = json!({"value":[{"id":2,"createdDate":"2026-01-02T00:00:00Z","sourceRefCommit":{"commitId":"abc"}}]});
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Passed);
}

#[test]
fn azure_stale_policy_with_a_build_stays_pending() {
    let mut policy = reviewer_policy("approved");
    policy["context"] = json!({"buildId":42});
    let mut case = AzureCase::new(json!([policy]), json!({"value":[]}));
    case.iterations = json!({"value":[{"id":2,"createdDate":"2026-01-02T00:00:00Z","sourceRefCommit":{"commitId":"abc"}}]});
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Timeout);
}

#[test]
fn azure_rejected_build_policy_fails_unless_its_run_was_cancelled() {
    for build in [
        Some(json!({"status":"inProgress"})),
        Some(json!({"status":"completed","result":"succeeded"})),
        Some(json!("odd")),
        None,
    ] {
        let mut policy = azure_policy("rejected", true);
        policy["context"] = json!({"buildId":42});
        let mut case = AzureCase::new(json!([policy]), json!({"value":[]}));
        case.build = build.clone();
        assert_eq!(
            run_azure(&case, 1).unwrap().outcome,
            Outcome::Failed,
            "{build:?}"
        );
    }
}

#[test]
fn azure_merge_conflicts_are_reported_and_other_merge_states_are_unknown() {
    for (merge, base) in [
        (json!("conflicts"), BaseState::Conflicts),
        (json!("succeeded"), BaseState::Unknown),
        (json!("queued"), BaseState::Unknown),
        (Value::Null, BaseState::Unknown),
    ] {
        let mut case = AzureCase::new(json!([azure_policy("approved", true)]), json!({"value":[]}));
        case.merge_status = merge.clone();
        assert_eq!(run_azure(&case, 1).unwrap().base, base, "{merge}");
    }
}

#[test]
fn legacy_azure_urls_go_to_azure_and_ask_for_the_canonical_url() {
    assert!(is_azure_pr_url(
        "https://dev.azure.com/o/p/_git/r/pullrequest/1"
    ));
    assert!(is_azure_pr_url(
        "https://contoso.visualstudio.com/p/_git/r/pullrequest/1"
    ));
    assert!(!is_azure_pr_url("https://github.com/o/r/pull/7"));
    assert!(!is_azure_pr_url(
        "https://evil.example/contoso.visualstudio.com/p"
    ));
    let error = wait(
        Some("https://contoso.visualstudio.com/p/_git/r/pullrequest/1"),
        &Options::default(),
        &AzureScript,
        &FakeClock::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        error.0.contains(
            "https://dev.azure.com/<organization>/<project>/_git/<repository>/pullrequest/<id>"
        ),
        "{}",
        error.0
    );
}

#[test]
fn azure_disabled_policy_is_ignored() {
    let mut disabled = azure_policy("rejected", true);
    disabled["configuration"]["isEnabled"] = json!(false);
    let case = AzureCase::new(
        json!([disabled, azure_policy("approved", true)]),
        json!({"value":[]}),
    );
    let waited = run_azure(&case, 1).unwrap();
    assert_eq!(waited.outcome, Outcome::Passed);
    assert_eq!(waited.checks.len(), 1);
}

#[test]
fn azure_pr_completed_or_abandoned_while_waiting_is_not_open() {
    for state in ["completed", "abandoned"] {
        let case = AzureCase::new(json!([azure_policy("queued", true)]), json!({"value":[]}));
        *case.pr_states.borrow_mut() = VecDeque::from(["active", "active", "active", state]);
        let waited = run_azure(&case, 60).unwrap();
        assert_eq!(waited.outcome, Outcome::NotOpen, "{state}");
        assert!(waited.checks.is_empty());
    }
}

#[test]
fn azure_fail_fast_returns_failed_while_another_policy_is_pending() {
    let case = AzureCase::new(
        json!([azure_policy("rejected", true), azure_policy("queued", true)]),
        json!({"value":[]}),
    );
    let options = Options {
        timeout: Duration::from_secs(3600),
        fail_fast: true,
        ..Options::default()
    };
    let waited = run_azure_with(&case, &options).unwrap();
    assert_eq!(waited.outcome, Outcome::Failed);
    assert!(waited.checks.iter().any(|check| check.bucket == "pending"));
}

#[test]
fn azure_cli_not_installed_or_returning_malformed_json_is_a_tool_problem() {
    let gh = FakeGh::default();
    gh.azure.borrow_mut().push_back(Err(GhError::NotInstalled));
    let error = run_azure_with(&gh, &Options::default()).unwrap_err();
    assert!(error.0.contains("install it"), "{}", error.0);

    let gh = FakeGh::default();
    gh.azure.borrow_mut().push_back(ok("{not json"));
    let error = run_azure_with(&gh, &Options::default()).unwrap_err();
    assert!(error.0.contains("invalid JSON"), "{}", error.0);
}

#[test]
fn a_just_pushed_commit_is_waited_for_before_its_checks_are_watched() {
    // Right after a push GitHub still reports the old head; it shows the pushed one on the next ask.
    let gh = FakeGh::script(
        vec![
            view("old111", "OPEN", "CLEAN"),
            view("new222", "OPEN", "CLEAN"),
        ],
        vec![checks(&[("build", "pass")])],
    );
    let options = Options {
        pushed_head: Some("new222".into()),
        ..Options::default()
    };
    let (result, _, clock) = run_wait_with(&gh, &options);
    let waited = result.unwrap();
    assert_eq!(waited.outcome, Outcome::Passed);
    assert_eq!(waited.head, "new222");
    assert_eq!(waited.new_head, None);
    assert_eq!(*clock.sleeps.borrow(), vec![Duration::from_secs(5)]);
}

#[test]
fn a_pushed_commit_the_pull_request_never_shows_is_given_up_on_after_thirty_seconds() {
    let gh = FakeGh::script(
        vec![view("old111", "OPEN", "CLEAN")],
        vec![checks(&[("build", "pass")])],
    );
    let options = Options {
        pushed_head: Some("new222".into()),
        ..Options::default()
    };
    let (result, _, clock) = run_wait_with(&gh, &options);
    // The wait then watches the head the pull request reports, as it did before.
    let waited = result.unwrap();
    assert_eq!(waited.outcome, Outcome::Passed);
    assert_eq!(waited.head, "old111");
    assert_eq!(clock.sleeps.borrow().len(), 6);
    assert_eq!(*clock.now.borrow(), Duration::from_secs(30));
}

#[test]
fn the_pushed_head_is_known_only_while_the_worktree_is_on_the_branch_on_origin() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "feature"]);
    git(&["commit", "-q", "--allow-empty", "-m", "one"]);
    // No origin branch yet.
    assert_eq!(pushed_head(dir.path(), "feature"), None);
    let one = git(&["rev-parse", "HEAD"]);
    git(&["update-ref", "refs/remotes/origin/feature", &one]);
    assert_eq!(pushed_head(dir.path(), "feature"), Some(one));
    // A local commit that is not pushed.
    git(&["commit", "-q", "--allow-empty", "-m", "two"]);
    assert_eq!(pushed_head(dir.path(), "feature"), None);
}
