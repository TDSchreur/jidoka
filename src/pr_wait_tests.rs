//! `jidoka pr wait` against a scripted `gh` and a clock that never sleeps.

use super::*;
use serde_json::{Value, json};
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

    if queue.len() > 1 {
        queue.pop_front().unwrap()
    } else {
        match queue.front().expect("the script has an answer") {
            Ok(output) => Ok(output.clone()),
            Err(GhError::NotInstalled) => Err(GhError::NotInstalled),
            Err(GhError::Other(text)) => Err(GhError::Other(text.clone())),
        }
    }
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
    assert!(
        clock
            .sleeps
            .borrow()
            .iter()
            .all(|d| *d == Duration::from_secs(15))
    );
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
        assert!(
            gh.calls
                .borrow()
                .iter()
                .all(|c| !c.starts_with("pr checks"))
        );
    }
}

// --- time, fail-fast and failure details ------------------------------------------------------------

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
    assert!(
        waited
            .checks
            .iter()
            .any(|c| c.name == "build" && c.bucket == "pending")
    );
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
    assert!(
        waited
            .checks
            .iter()
            .any(|c| c.name == "b" && c.bucket == "pending")
    );

    // Without it the wait goes on until every check has ended.
    let (result, _, clock) = run_wait(&script());
    assert_eq!(result.unwrap().outcome, Outcome::Failed);
    assert_eq!(clock.sleeps.borrow().len(), 2);
}

// --- rework 1 ---------------------------------------------------------------------------------------

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
fn azure_stale_iteration_cannot_pass() {
    let mut case = AzureCase::new(json!([azure_policy("approved", true)]), json!({"value":[]}));
    case.iterations = json!({"value":[{"id":2,"sourceRefCommit":{"commitId":"old"}}]});
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Timeout);
}

#[test]
fn azure_stale_build_policy_cannot_pass() {
    let mut policy = azure_policy("approved", true);
    policy["startedDate"] = json!("2026-01-01T00:00:00Z");
    let mut case = AzureCase::new(json!([policy]), json!({"value":[]}));
    case.iterations = json!({"value":[{"id":2,"createdDate":"2026-01-02T00:00:00Z","sourceRefCommit":{"commitId":"abc"}}]});
    assert_eq!(run_azure(&case, 1).unwrap().outcome, Outcome::Timeout);
}

fn reviewer_policy(state: &str) -> Value {
    json!({"status": state, "startedDate":"2026-01-01T00:00:00Z", "configuration":{"isEnabled":true,"isBlocking":true,"type":{"id":"fa4e907d-c16b-4a4c-9dfa-4906e5d171dd","displayName":"Minimum number of reviewers"}}})
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
