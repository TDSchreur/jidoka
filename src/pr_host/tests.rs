//! The shared adapter suite: the same cases run against the GitHub and the Azure Repos adapter, through
//! `Hosts`, with a scripted `gh` and `az`. Only these tests script the hosts' tools.

use super::*;
use crate::pr_wait::BaseState;
use serde_json::{Value, json};
use std::cell::RefCell;

/// Answers the calls from a script and records them, as `gh <args>` or `az <args>`. An error answer
/// stands for a tool that is not installed.
/// How a script answers a call; an error stands for a tool that is not installed.
type Answer = Box<dyn Fn(&str) -> Result<ToolOutput, String>>;

struct Script {
    answer: Answer,
    calls: RefCell<Vec<String>>,
}

impl Script {
    fn printing(stdout: String) -> Self {
        Self::answering(Ok(ToolOutput {
            code: Some(0),
            stdout,
            stderr: String::new(),
        }))
    }

    /// Answers every call with `answer`.
    fn answering(answer: Result<ToolOutput, String>) -> Self {
        Self {
            answer: Box::new(move |_| answer.clone()),
            calls: RefCell::new(Vec::new()),
        }
    }

    /// Answers each call with what `print` gives for it.
    fn printing_for(print: impl Fn(&str) -> String + 'static) -> Self {
        Self {
            answer: Box::new(move |call| {
                Ok(ToolOutput {
                    code: Some(0),
                    stdout: print(call),
                    stderr: String::new(),
                })
            }),
            calls: RefCell::new(Vec::new()),
        }
    }

    /// A tool that is not installed.
    fn missing() -> Self {
        Self::answering(Err(String::new()))
    }

    fn reply(&self, call: String) -> Result<ToolOutput, ToolError> {
        let answer = (self.answer)(&call);
        self.calls.borrow_mut().push(call);
        answer.map_err(|_| ToolError::NotInstalled)
    }
}

impl Tools for Script {
    fn gh(&self, dir: Option<&Path>, args: &[&str]) -> Result<ToolOutput, ToolError> {
        match dir {
            Some(dir) => self.reply(format!("gh (in {}) {}", dir.display(), args.join(" "))),
            None => self.reply(format!("gh {}", args.join(" "))),
        }
    }

    fn az(&self, args: &[&str]) -> Result<ToolOutput, ToolError> {
        self.reply(format!("az {}", args.join(" ")))
    }
}

/// One host as the shared cases see it.
struct Case {
    url: &'static str,
    /// The one call that reads the state.
    state_call: &'static str,
    /// What the host's tool prints for a pull request in `state` from branch `feature/x` at commit
    /// `ABCD1234`; `PrState::Unknown` stands for a state the host reports that Jidoka does not know.
    print: fn(PrState) -> String,
    /// What the tool prints to stderr, and its exit status, when it is not signed in.
    signed_out: (i32, &'static str),
    signed_out_message: &'static str,
    not_installed_message: &'static str,
    /// The origin remote of the host's repository.
    remote: &'static str,
    /// The URL of a pull request in that repository, without its number.
    pr_url: &'static str,
    /// What the host's tool prints for the open pull requests of `feature/x` into `develop`, as
    /// `(number, from a fork)`.
    list: fn(&[(u64, bool)]) -> String,
    /// The one call that finds the open pull request of `feature/x`, run in the worktree `wt`.
    find_call: &'static str,
    /// What the host's tool prints when it created pull request `number`.
    created: fn(u64) -> String,
    /// The start of the one call that creates the pull request, up to its description.
    create_call: &'static str,
    /// What the host's tool prints for `call` while the checks of a pull request in `state` at commit
    /// `abc1234` are watched: `checks` as `(name, bucket)`, and whether the branch conflicts with its base.
    watch: WatchPrint,
    /// The start of the call that reads the pull request in a watch.
    watch_view: &'static str,
    /// The base state the host reports for a branch without conflicts.
    clean_base: BaseState,
}

/// What a host's tool prints for a call while checks are watched; see `Case::watch`.
type WatchPrint = fn(PrState, &[(&str, &str)], bool, &str) -> String;

/// GitHub's `state` and `isDraft` for `state`; `PrState::Unknown` stands for a state Jidoka does not know.
fn github_state(state: PrState) -> (&'static str, bool) {
    match state {
        PrState::Open => ("OPEN", false),
        PrState::Draft => ("OPEN", true),
        PrState::Merged => ("MERGED", false),
        PrState::Closed => ("CLOSED", false),
        PrState::Unknown => ("WEIRD", false),
    }
}

/// Azure's `status` and `isDraft` for `state`; `PrState::Unknown` stands for a status Jidoka does not know.
fn azure_state(state: PrState) -> (&'static str, bool) {
    match state {
        PrState::Open => ("active", false),
        PrState::Draft => ("active", true),
        PrState::Merged => ("completed", false),
        PrState::Closed => ("abandoned", false),
        PrState::Unknown => ("weird", false),
    }
}

fn github() -> Case {
    Case {
        url: "https://github.com/o/r/pull/7",
        state_call: "gh pr view https://github.com/o/r/pull/7 --json state,isDraft",
        print: |state| {
            let (state, draft) = github_state(state);
            json!({"state": state, "isDraft": draft, "headRefOid": "ABCD1234", "headRefName": "feature/x"})
                .to_string()
        },
        signed_out: (
            4,
            "To get started with GitHub CLI, please run:  gh auth login",
        ),
        signed_out_message: "The GitHub CLI is not signed in. Run `gh auth login` and try again.",
        not_installed_message: "The GitHub CLI (`gh`) is not installed or not on PATH. Install it from https://cli.github.com, run `gh auth login`, and try again.",
        remote: "https://github.com/o/r.git",
        pr_url: "https://github.com/o/r/pull/",
        list: |prs| {
            let items: Vec<_> = prs
                .iter()
                .map(|(number, fork)| {
                    json!({
                        "url": format!("https://github.com/o/r/pull/{number}"),
                        "baseRefName": "develop",
                        "isCrossRepository": fork
                    })
                })
                .collect();
            json!(items).to_string()
        },
        find_call: "gh (in wt) pr list --head feature/x --state open --json url,baseRefName,isCrossRepository",
        created: |number| {
            format!(
                "Creating pull request for feature/x into develop\n\nhttps://github.com/o/r/pull/{number}\n"
            )
        },
        create_call: "gh (in wt) pr create --base develop --head feature/x --title Add the thing --body-file ",
        watch: |state, checks, conflicts, call| {
            if call.starts_with("gh pr checks") {
                let items: Vec<_> = checks
                    .iter()
                    .map(|(name, bucket)| {
                        json!({"name": name, "bucket": bucket, "state": "X", "link": "https://github.com/o/r/actions/runs/1/job/2"})
                    })
                    .collect();
                return json!(items).to_string();
            }
            let (state, draft) = github_state(state);
            let merge = if conflicts { "DIRTY" } else { "CLEAN" };
            json!({"state": state, "isDraft": draft, "headRefOid": "abc1234", "mergeStateStatus": merge})
                .to_string()
        },
        watch_view: "gh pr view",
        clean_base: BaseState::UpToDate,
    }
}

fn azure() -> Case {
    Case {
        url: "https://dev.azure.com/org/Proj/_git/Repo/pullrequest/535",
        state_call: "az repos pr show --id 535 --organization https://dev.azure.com/org --output json --detect false",
        print: |state| {
            let (status, draft) = azure_state(state);
            json!({
                "pullRequestId": 535,
                "status": status,
                "isDraft": draft,
                "repository": {"name": "Repo", "project": {"name": "Proj"}},
                "forkSource": null,
                "sourceRefName": "refs/heads/feature/x",
                "lastMergeSourceCommit": {"commitId": "ABCD1234"}
            })
            .to_string()
        },
        signed_out: (
            1,
            "ERROR: Please run 'az login' to setup account. secret-token-123",
        ),
        signed_out_message: "Azure CLI could not read the pull request; check Azure authentication with az login or az devops login.",
        not_installed_message: "Azure CLI (`az`) is not installed; install it and the azure-devops extension.",
        remote: "https://dev.azure.com/org/Proj/_git/Repo",
        pr_url: "https://dev.azure.com/org/Proj/_git/Repo/pullrequest/",
        list: |prs| {
            let items: Vec<_> = prs
                .iter()
                .map(|(number, fork)| {
                    json!({
                        "pullRequestId": number,
                        "targetRefName": "refs/heads/develop",
                        "forkSource": if *fork { json!({"name": "refs/heads/feature/x"}) } else { json!(null) }
                    })
                })
                .collect();
            json!(items).to_string()
        },
        find_call: "az repos pr list --organization https://dev.azure.com/org --project Proj --repository Repo --source-branch feature/x --status active --output json --detect false",
        created: |number| json!({"pullRequestId": number}).to_string(),
        create_call: "az repos pr create --organization https://dev.azure.com/org --project Proj --repository Repo --source-branch feature/x --target-branch develop --title=Add the thing --description @",
        watch: |state, checks, conflicts, call| {
            let mut azure = AzureWatch::new(checks);
            azure.status = azure_state(state);
            azure.merge_status = if conflicts { "conflicts" } else { "succeeded" };
            azure.print(call)
        },
        watch_view: "az repos pr show",
        // A merge that succeeds does not prove that the branch is up to date.
        clean_base: BaseState::Unknown,
    }
}

/// What `az` prints while the checks of Azure pull request 535 at commit `abc1234` are watched.
struct AzureWatch {
    status: (&'static str, bool),
    merge_status: &'static str,
    policies: Value,
    iterations: Value,
    iteration_statuses: Value,
}

impl AzureWatch {
    /// An active pull request without policies, whose current iteration has `checks` as statuses.
    fn new(checks: &[(&str, &str)]) -> Self {
        let statuses: Vec<_> = checks
            .iter()
            .enumerate()
            .map(|(index, (name, bucket))| {
                let state = match *bucket {
                    "pass" => "succeeded",
                    "fail" => "failed",
                    "cancel" => "canceled",
                    "skipping" => "notApplicable",
                    _ => "pending",
                };
                json!({"id": index + 1, "iterationId": 1, "state": state, "context": {"name": name}})
            })
            .collect();
        Self {
            status: ("active", false),
            merge_status: "succeeded",
            policies: json!([]),
            iterations: json!({"value": [{"id": 1, "createdDate": "2026-01-01T00:00:00Z", "sourceRefCommit": {"commitId": "abc1234"}}]}),
            iteration_statuses: json!({"value": statuses}),
        }
    }

    fn print(&self, call: &str) -> String {
        let value = if call.starts_with("az repos pr show") {
            json!({
                "pullRequestId": 535,
                "status": self.status.0,
                "isDraft": self.status.1,
                "mergeStatus": self.merge_status,
                "repository": {"id": "repo-id", "name": "Repo", "project": {"name": "Proj"}},
                "forkSource": null,
                "sourceRefName": "refs/heads/feature/x",
                "lastMergeSourceCommit": {"commitId": "abc1234"}
            })
        } else if call.starts_with("az repos ref list") {
            json!([{"name": "refs/heads/feature/x", "objectId": "abc1234"}])
        } else if call.starts_with("az repos pr policy list") {
            self.policies.clone()
        } else if call.contains(" pullRequestIterations ") {
            self.iterations.clone()
        } else if call.contains(" pullRequestIterationStatuses ") {
            self.iteration_statuses.clone()
        } else if call.contains(" pullRequestStatuses ") {
            json!({"value": []})
        } else {
            panic!("unexpected call {call}")
        };
        value.to_string()
    }
}

const KNOWN: [PrState; 4] = [
    PrState::Open,
    PrState::Draft,
    PrState::Merged,
    PrState::Closed,
];

#[test]
fn both_hosts_read_open_draft_merged_and_closed_in_one_call() {
    for case in [github(), azure()] {
        for state in KNOWN {
            let tools = Script::printing((case.print)(state));
            assert_eq!(
                Hosts::new(&tools).state(case.url),
                Ok(state),
                "{}",
                case.url
            );
            assert_eq!(*tools.calls.borrow(), [case.state_call]);
        }
    }
}

#[test]
fn both_hosts_read_the_head_with_its_state_in_one_call() {
    for case in [github(), azure()] {
        for state in KNOWN {
            let tools = Script::printing((case.print)(state));
            let head = Hosts::new(&tools).head(case.url).unwrap();
            let expected = Head {
                state,
                commit: "abcd1234".into(),
                branch: "feature/x".into(),
            };
            assert_eq!(head, expected, "{}", case.url);
            assert_eq!(tools.calls.borrow().len(), 1, "{}", case.url);
        }
    }
}

#[test]
fn a_state_the_host_reports_but_jidoka_does_not_know_is_a_problem() {
    for case in [github(), azure()] {
        let tools = Script::printing((case.print)(PrState::Unknown));
        let ToolProblem(message) = Hosts::new(&tools).state(case.url).unwrap_err();
        assert!(message.contains("unknown pull request"), "{message}");
        assert!(Hosts::new(&tools).head(case.url).is_err(), "{}", case.url);
    }
}

#[test]
fn a_signed_out_tool_says_how_to_sign_in_without_echoing_its_output() {
    for case in [github(), azure()] {
        let (code, stderr) = case.signed_out;
        let tools = Script::answering(Ok(ToolOutput {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.into(),
        }));
        let problem = Hosts::new(&tools).state(case.url).unwrap_err();
        assert_eq!(problem.0, case.signed_out_message);
    }
}

#[test]
fn a_missing_tool_says_how_to_install_it() {
    for case in [github(), azure()] {
        let tools = Script::missing();
        let problem = Hosts::new(&tools).head(case.url).unwrap_err();
        assert_eq!(problem.0, case.not_installed_message);
    }
}

#[test]
fn only_http_links_to_github_or_azure_pull_requests_reach_a_tool() {
    for url in [
        "--repo=evil",
        "file:///tmp/pull/7",
        "https://example.com/o/r/issues/7",
    ] {
        let tools = Script::missing();
        assert!(Hosts::new(&tools).state(url).is_err(), "{url}");
        assert!(tools.calls.borrow().is_empty(), "{url}");
    }
}

/// The origin of the host's repository, as `detect_host` reads it.
fn origin(case: &Case) -> Origin {
    detect_host(&|args| (args == ["remote", "get-url", "origin"]).then(|| case.remote.to_string()))
        .unwrap()
}

#[test]
fn both_hosts_find_the_open_pull_request_of_a_branch_but_not_one_from_a_fork() {
    for case in [github(), azure()] {
        let origin = origin(&case);
        let tools = Script::printing((case.list)(&[(6, true), (7, false)]));
        let found = Hosts::new(&tools)
            .find_open(Path::new("wt"), &origin, "feature/x")
            .unwrap();
        let expected = OpenPr {
            url: format!("{}7", case.pr_url),
            base: Some("develop".into()),
        };
        assert_eq!(found, Some(expected), "{}", case.remote);
        assert_eq!(*tools.calls.borrow(), [case.find_call]);

        let tools = Script::printing((case.list)(&[(6, true)]));
        let found = Hosts::new(&tools)
            .find_open(Path::new("wt"), &origin, "feature/x")
            .unwrap();
        assert_eq!(found, None, "{}", case.remote);
    }
}

#[test]
fn both_hosts_create_a_pull_request_and_return_its_url() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("body.md");
    std::fs::write(&body, "Body text").unwrap();
    let pr = NewPr {
        branch: "feature/x",
        base: "develop",
        title: "Add the thing",
        body_file: &body,
    };
    for case in [github(), azure()] {
        let origin = origin(&case);
        let tools = Script::printing((case.created)(8));
        let hosts = Hosts::new(&tools);
        assert_eq!(hosts.check_new(&origin, &pr), Ok(()), "{}", case.remote);
        let url = hosts.create(Path::new("wt"), &origin, &pr);
        assert_eq!(url, Ok(format!("{}8", case.pr_url)));
        let calls = tools.calls.borrow();
        assert_eq!(calls.len(), 1, "{}", case.remote);
        assert!(calls[0].starts_with(case.create_call), "{}", calls[0]);
    }
}

/// The snapshot a watch reads from the host of `case`, and the calls it made.
fn snapshot(
    case: &Case,
    state: PrState,
    checks: &'static [(&'static str, &'static str)],
    conflicts: bool,
) -> (watch::Snapshot, Vec<String>) {
    let print = case.watch;
    let tools = Script::printing_for(move |call| print(state, checks, conflicts, call));
    let adapter = Hosts::<Script>::adapter(case.url).unwrap();
    let snapshot = adapter.snapshot(&tools, case.url).unwrap();
    (snapshot, tools.calls.take())
}

/// The checks of `snapshot` as `(name, bucket)`.
fn buckets(snapshot: &watch::Snapshot) -> Vec<(&str, &str)> {
    snapshot
        .checks
        .iter()
        .map(|check| (check.name.as_str(), check.bucket.as_str()))
        .collect()
}

#[test]
fn both_hosts_read_a_watch_snapshot_with_the_pull_request_before_and_after_its_checks() {
    const CHECKS: &[(&str, &str)] = &[
        ("build", "pass"),
        ("deploy", "cancel"),
        ("lint", "fail"),
        ("test", "pending"),
    ];
    for case in [github(), azure()] {
        for state in [PrState::Open, PrState::Draft] {
            let (snapshot, calls) = snapshot(&case, state, CHECKS, false);
            assert_eq!(snapshot.state, state, "{}", case.url);
            assert_eq!(snapshot.head, "abc1234");
            assert_eq!(buckets(&snapshot), CHECKS, "{}", case.url);
            assert!(snapshot.applies);
            assert_eq!(snapshot.base, Some(case.clean_base), "{}", case.url);
            let views = calls.iter().filter(|c| c.starts_with(case.watch_view));
            assert_eq!(views.count(), 2, "{calls:?}");
            assert!(calls.first().unwrap().starts_with(case.watch_view));
        }
        let (snapshot, _) = snapshot(&case, PrState::Open, CHECKS, true);
        assert_eq!(snapshot.base, Some(BaseState::Conflicts), "{}", case.url);
    }
}

#[test]
fn both_hosts_report_a_pull_request_without_checks_as_one_no_check_applies_to() {
    for case in [github(), azure()] {
        let (snapshot, _) = snapshot(&case, PrState::Open, &[], false);
        assert!(snapshot.checks.is_empty(), "{}", case.url);
        assert!(!snapshot.applies, "{}", case.url);
    }
}

#[test]
fn both_hosts_read_no_checks_of_a_merged_or_closed_pull_request() {
    for case in [github(), azure()] {
        for state in [PrState::Merged, PrState::Closed] {
            let (snapshot, calls) = snapshot(&case, state, &[("build", "pass")], false);
            assert_eq!(snapshot.state, state, "{}", case.url);
            assert_eq!(snapshot.head, "abc1234");
            assert!(snapshot.checks.is_empty());
            assert_eq!(calls.len(), 1, "{calls:?}");
        }
    }
}

const AZURE_PR: &str = "https://dev.azure.com/org/Proj/_git/Repo/pullrequest/535";

/// The checks of the Azure pull request `azure` prints, as `(name, bucket)`.
fn azure_checks(azure: AzureWatch) -> Vec<(String, String)> {
    let tools = Script::printing_for(move |call| azure.print(call));
    let snapshot = azure::Azure.snapshot(&tools, AZURE_PR).unwrap();
    buckets(&snapshot)
        .into_iter()
        .map(|(name, bucket)| (name.to_string(), bucket.to_string()))
        .collect()
}

fn azure_policy(name: &str, started: &str) -> Value {
    json!({"status": "approved", "startedDate": started, "configuration": {"isEnabled": true, "isBlocking": true, "type": {"displayName": name}}})
}

#[test]
fn azure_counts_a_build_only_for_the_current_iteration() {
    let check = |name: &str, bucket: &str| vec![(name.to_string(), bucket.to_string())];
    let mut current = AzureWatch::new(&[]);
    current.policies = json!([azure_policy("Build", "2026-01-01T00:00:01Z")]);
    assert_eq!(azure_checks(current), check("Build", "pass"));

    // Azure has not caught up with the head yet.
    let mut stale_iteration = AzureWatch::new(&[]);
    stale_iteration.policies = json!([azure_policy("Build", "2026-01-01T00:00:01Z")]);
    stale_iteration.iterations =
        json!({"value": [{"id": 1, "sourceRefCommit": {"commitId": "old"}}]});
    assert_eq!(
        azure_checks(stale_iteration),
        check("Azure is updating the current PR iteration", "pending")
    );

    // A build that started before the current iteration says nothing about it, approved or not.
    let mut stale_build = AzureWatch::new(&[]);
    stale_build.policies = json!([azure_policy("Build", "2025-12-31T00:00:00Z")]);
    assert_eq!(azure_checks(stale_build), check("Build", "pending"));

    let mut reviewers = azure_policy("Minimum number of reviewers", "2025-12-31T00:00:00Z");
    reviewers["context"] = json!({"buildId": 42});
    let mut stale_reviewers = AzureWatch::new(&[]);
    stale_reviewers.policies = json!([reviewers]);
    assert_eq!(
        azure_checks(stale_reviewers),
        check("Minimum number of reviewers", "pending")
    );
}

#[test]
fn azure_watch_reports_a_bounded_failed_task_log_and_keeps_checks_without_logs() {
    let mut azure = AzureWatch::new(&[("External scan", "fail")]);
    let rejected = |name: &str, build_id: u64| {
        let mut policy = azure_policy(name, "2026-01-01T00:00:01Z");
        policy["status"] = json!("rejected");
        policy["context"] = json!({"buildId": build_id});
        policy
    };
    azure.policies = json!([
        rejected("Compile", 42),
        rejected("Compile again", 42),
        rejected("Integration", 43),
        rejected("Package", 44),
        rejected("Publish", 45),
        rejected("Cancelled build", 46),
    ]);

    azure.iteration_statuses["value"][0]["targetUrl"] =
        json!("https://dev.azure.com/org/Proj/_build/results?buildId=47");

    let tools = Script {
        answer: Box::new(move |call| {
            let output = if call.contains("--area build ") {
                failed_azure_log_output(call)
            } else if call.starts_with("az pipelines runs show") {
                let result = if call.contains("--id 46 ") {
                    "canceled"
                } else {
                    "failed"
                };
                success(json!({"status": "completed", "result": result}).to_string())
            } else {
                success(azure.print(call))
            };
            Ok(output)
        }),
        calls: RefCell::new(Vec::new()),
    };
    let clock = crate::pr_wait::RealClock(std::time::Instant::now());
    let waited = Hosts::new(&tools)
        .watch(
            AZURE_PR,
            &Options::default(),
            &clock,
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(waited.exit_code, 2);
    let failures = &waited.failures;
    assert_eq!(
        failures
            .iter()
            .map(|failure| failure.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Compile",
            "Compile again",
            "Integration",
            "Package",
            "Publish",
            "External scan"
        ]
    );
    assert_eq!(
        failures[0].link,
        "https://dev.azure.com/org/Proj/_build/results?buildId=42"
    );
    assert_eq!(
        failures[2].link,
        "https://dev.azure.com/org/Proj/_build/results?buildId=43"
    );
    assert_eq!(failures[5].link, AZURE_PR);
    let excerpt = failures[0].excerpt.as_deref().unwrap();
    assert!(excerpt.len() <= 4096);
    assert!(excerpt.ends_with("compiler exited with code 1"));
    let line_limited = failures[3].excerpt.as_deref().unwrap();
    assert_eq!(line_limited.lines().count(), 40);
    assert!(line_limited.starts_with("output 1\n"));
    assert!(line_limited.ends_with("output 40"));
    for index in [1, 2, 4, 5] {
        assert_eq!(failures[index].excerpt, None);
    }
    assert!(crate::pr_wait::render(&waited).contains("compiler exited with code 1"));
    let calls = tools.calls.borrow();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.contains("--resource timeline "))
            .count(),
        3
    );
}

fn success(stdout: String) -> ToolOutput {
    ToolOutput {
        code: Some(0),
        stdout,
        stderr: String::new(),
    }
}

/// A build lookup for an inaccessible log or for a failed task with a large log.
fn failed_azure_log_output(call: &str) -> ToolOutput {
    if call.contains("buildId=43 ") {
        return ToolOutput {
            code: Some(1),
            stdout: String::new(),
            stderr: "Access denied".into(),
        };
    }
    let long_line = call.contains("buildId=42 ");
    assert!(
        long_line || call.contains("buildId=44 "),
        "unexpected build log request {call}"
    );
    let value = if call.contains("--resource timeline ") {
        json!({"records": [
            {"type": "Job", "result": "failed", "log": {"id": 2}},
            {"type": "Task", "result": "succeeded", "log": {"id": 5}},
            {"type": "Task", "result": "failed", "log": {"id": 8}}
        ]})
    } else if call.contains("--resource logs ") && call.contains(" logId=8 ") {
        assert!(
            call.contains("--query-parameters startLine=160 endLine=200 "),
            "{call}"
        );
        let mut lines: Vec<_> = (0..=40).map(|line| format!("output {line}")).collect();
        if long_line {
            lines[40] = format!("{} compiler exited with code 1", "🦀".repeat(1500));
        }
        json!({"value": lines})
    } else {
        assert!(
            call.contains("--resource logs ") && !call.contains("logId="),
            "{call}"
        );
        json!({"value": [{"id": 2, "lineCount": 300}, {"id": 8, "lineCount": 200}]})
    };
    success(value.to_string())
}
