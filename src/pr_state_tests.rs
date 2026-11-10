//! Pull request state against a scripted `gh`, and the refresh pass against a real run store.

use super::*;
use crate::pr_wait::{GhError, GhOutput};
use crate::run::{NewRun, RunStore, UpdateRun};
use crate::test_support::TestDir;
use std::sync::Mutex;

const PR: &str = "https://github.com/o/r/pull/7";

/// Answers every `gh` call with the next scripted answer (the last one repeats) and records the calls.
struct ScriptedGh {
    answers: Mutex<Vec<Result<GhOutput, GhError>>>,
    calls: Mutex<Vec<String>>,
}

impl ScriptedGh {
    fn new(answers: Vec<Result<GhOutput, GhError>>) -> Self {
        Self {
            answers: Mutex::new(answers),
            calls: Mutex::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ScriptedGh {
    fn answer(&self, call: String) -> Result<GhOutput, GhError> {
        self.calls.lock().unwrap().push(call);
        let mut answers = self.answers.lock().unwrap();
        if answers.len() > 1 {
            answers.remove(0)
        } else {
            match &answers[0] {
                Ok(output) => Ok(output.clone()),
                Err(GhError::NotInstalled) => Err(GhError::NotInstalled),
                Err(GhError::Other(text)) => Err(GhError::Other(text.clone())),
            }
        }
    }
}

impl Gh for ScriptedGh {
    fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        self.answer(args.join(" "))
    }
    fn az(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        self.answer(format!("az {}", args.join(" ")))
    }
}

fn view(state: &str, draft: bool) -> Result<GhOutput, GhError> {
    Ok(GhOutput {
        code: Some(0),
        stdout: format!(r#"{{"state":"{state}","isDraft":{draft}}}"#),
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

#[test]
fn a_github_pull_request_reads_as_open_draft_merged_or_closed() {
    // `gh pr view --json state,isDraft` reports OPEN, MERGED or CLOSED, and isDraft for an open draft.
    for (state, draft, expected) in [
        ("OPEN", false, PrState::Open),
        ("OPEN", true, PrState::Draft),
        ("MERGED", false, PrState::Merged),
        ("CLOSED", false, PrState::Closed),
    ] {
        let gh = ScriptedGh::new(vec![view(state, draft)]);
        assert_eq!(read(&gh, PR).unwrap(), expected, "{state} draft={draft}");
        assert_eq!(gh.calls(), [format!("pr view {PR} --json state,isDraft")]);
    }
}

#[test]
fn problems_say_what_to_do() {
    let missing = ScriptedGh::new(vec![Err(GhError::NotInstalled)]);
    assert!(read(&missing, PR).unwrap_err().0.contains("gh auth login"));
    let signed_out = ScriptedGh::new(vec![failed(
        4,
        "To get started with GitHub CLI, please run:  gh auth login",
    )]);
    assert!(read(&signed_out, PR)
        .unwrap_err()
        .0
        .contains("not signed in"));
    let odd = ScriptedGh::new(vec![view("SOMETHING", false)]);
    assert!(read(&odd, PR).unwrap_err().0.contains("SOMETHING"));
    let not_json = ScriptedGh::new(vec![Ok(GhOutput {
        code: Some(0),
        stdout: "nope".into(),
        stderr: String::new(),
    })]);
    assert!(read(&not_json, PR).unwrap_err().0.contains("not JSON"));
    // Another host has no pull request state here, and no tool is asked.
    let other = ScriptedGh::new(vec![view("OPEN", false)]);
    let problem = read(&other, "https://git.example.com/o/r/pulls/3")
        .unwrap_err()
        .0;
    assert!(problem.contains("GitHub and Azure Repos"), "{problem}");
    assert!(other.calls().is_empty());
    // A GitHub Enterprise pull request has the same shape, so gh is asked.
    let enterprise = ScriptedGh::new(vec![view("MERGED", false)]);
    assert_eq!(
        read(&enterprise, "https://github.example.com/o/r/pull/9").unwrap(),
        PrState::Merged
    );
    // A link to a tab or a comment of a GitHub pull request reads the pull request itself.
    for link in [
        "https://github.com/o/r/pull/7/files",
        "https://github.com/o/r/pull/7#issuecomment-123",
        "https://github.com/o/r/pull/7?w=1",
    ] {
        let tabbed = ScriptedGh::new(vec![view("OPEN", false)]);
        assert_eq!(read(&tabbed, link).unwrap(), PrState::Open, "{link}");
        assert_eq!(
            tabbed.calls(),
            [format!("pr view {PR} --json state,isDraft")]
        );
    }
    // A repository may itself be called pull: the pull request is the last /pull/<number> in the link.
    let named_pull = ScriptedGh::new(vec![view("MERGED", false)]);
    assert_eq!(
        read(&named_pull, "https://github.com/o/pull/pull/7/files").unwrap(),
        PrState::Merged
    );
    assert_eq!(
        named_pull.calls(),
        ["pr view https://github.com/o/pull/pull/7 --json state,isDraft"]
    );
    // Only http(s) URLs are pull request links; anything else is never handed to a tool.
    let unused = ScriptedGh::new(vec![view("OPEN", false)]);
    assert!(read(&unused, "--help").is_err());
    assert!(unused.calls().is_empty());
}

fn store() -> (TestDir, RunStore) {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    (dir, store)
}

fn run_with_pr(store: &RunStore, pr: &str) -> String {
    let run = store
        .create(NewRun {
            title: "t".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        })
        .unwrap();
    let update: UpdateRun = serde_json::from_str(&format!(
        r#"{{"stage":"finish","status":"running","pr":"{pr}"}}"#
    ))
    .unwrap();
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
fn a_refresh_stores_the_state_with_its_url_and_time() {
    let (_dir, store) = store();
    let id = run_with_pr(&store, PR);
    let gh = ScriptedGh::new(vec![view("OPEN", false)]);
    assert_eq!(refresh(&store, &gh, 1_000).unwrap(), 1);
    assert_eq!(
        stored(&store, &id),
        Some(PrStatus {
            url: PR.into(),
            state: PrState::Open,
            reason: None,
            checked_at: 1_000,
        })
    );
}

#[test]
fn merged_is_final_and_others_are_checked_again_after_a_minute() {
    let (_dir, store) = store();
    run_with_pr(&store, PR);
    let gh = ScriptedGh::new(vec![view("OPEN", false), view("MERGED", false)]);
    refresh(&store, &gh, 0).unwrap();
    // Within the minute nothing is asked again.
    assert_eq!(refresh(&store, &gh, 59_999).unwrap(), 0);
    // After it, the open one is, and it has been merged.
    assert_eq!(refresh(&store, &gh, 60_000).unwrap(), 1);
    // Merged is never asked again, however much later.
    assert_eq!(refresh(&store, &gh, 10_000_000).unwrap(), 0);
    assert_eq!(gh.calls().len(), 2);
}

#[test]
fn a_problem_is_stored_as_unknown_with_its_reason_and_tried_again() {
    let (_dir, store) = store();
    let id = run_with_pr(&store, PR);
    let gh = ScriptedGh::new(vec![Err(GhError::NotInstalled), view("CLOSED", false)]);
    refresh(&store, &gh, 0).unwrap();
    let unknown = stored(&store, &id).unwrap();
    assert_eq!(unknown.state, PrState::Unknown);
    assert!(unknown.reason.unwrap().contains("not installed"));
    refresh(&store, &gh, 60_000).unwrap();
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
    let gh = ScriptedGh::new(vec![view("MERGED", false), view("OPEN", true)]);
    assert_eq!(refresh(&store, &gh, 0).unwrap(), 1);
    // The Run now records another pull request: its stored state is for the old URL, so it is read again.
    let other = "https://github.com/o/r/pull/8";
    let update: UpdateRun = serde_json::from_str(&format!(
        r#"{{"stage":"finish","status":"running","pr":"{other}"}}"#
    ))
    .unwrap();
    store.update(&id, update).unwrap();
    assert_eq!(refresh(&store, &gh, 1).unwrap(), 1);
    let status = stored(&store, &id).unwrap();
    assert_eq!((status.url.as_str(), status.state), (other, PrState::Draft));
}

#[test]
fn callers_cannot_set_the_state() {
    let attempt = serde_json::from_str::<UpdateRun>(r#"{"pr_status":{"state":"merged"}}"#);
    assert!(attempt.is_err());
}

const AZURE_PR: &str = "https://dev.azure.com/org/Proj/_git/Repo/pullrequest/535";

/// What `az repos pr show` prints for pull request 535 of Proj/Repo, cut to the fields that matter here.
fn azure(status: &str, draft: bool) -> Result<GhOutput, GhError> {
    Ok(GhOutput {
        code: Some(0),
        stdout: format!(
            r#"{{"pullRequestId":535,"status":"{status}","isDraft":{draft},"repository":{{"name":"Repo","project":{{"name":"Proj"}}}},"forkSource":null}}"#
        ),
        stderr: String::new(),
    })
}

#[test]
fn an_azure_repos_pull_request_reads_as_open_draft_merged_or_closed() {
    // Azure Repos reports active, completed or abandoned, and isDraft for an active draft.
    for (status, draft, expected) in [
        ("active", false, PrState::Open),
        ("active", true, PrState::Draft),
        ("completed", false, PrState::Merged),
        ("abandoned", false, PrState::Closed),
    ] {
        let az = ScriptedGh::new(vec![azure(status, draft)]);
        assert_eq!(
            read(&az, AZURE_PR).unwrap(),
            expected,
            "{status} draft={draft}"
        );
        assert_eq!(
            az.calls(),
            ["az repos pr show --id 535 --organization https://dev.azure.com/org --output json --detect false"]
        );
    }
}

#[test]
fn azure_problems_say_what_to_do() {
    let missing = ScriptedGh::new(vec![Err(GhError::NotInstalled)]);
    assert!(read(&missing, AZURE_PR)
        .unwrap_err()
        .0
        .contains("azure-devops extension"));
    let signed_out = ScriptedGh::new(vec![failed(1, "Please run 'az login' to setup account.")]);
    assert!(read(&signed_out, AZURE_PR)
        .unwrap_err()
        .0
        .contains("az login"));
    let odd = ScriptedGh::new(vec![azure("notSet", false)]);
    assert!(read(&odd, AZURE_PR).unwrap_err().0.contains("notSet"));
    // A pull request from another repository than the URL names is not trusted.
    let elsewhere = ScriptedGh::new(vec![Ok(GhOutput {
        code: Some(0),
        stdout: r#"{"pullRequestId":535,"status":"active","isDraft":false,"repository":{"name":"Other","project":{"name":"Proj"}},"forkSource":null}"#.into(),
        stderr: String::new(),
    })]);
    assert!(read(&elsewhere, AZURE_PR)
        .unwrap_err()
        .0
        .contains("does not match"));
    // Azure output is never echoed: it can hold credentials.
    let secret = ScriptedGh::new(vec![failed(1, "token=abc123 rejected")]);
    assert!(!read(&secret, AZURE_PR).unwrap_err().0.contains("abc123"));
}

#[test]
fn the_head_of_a_pull_request_is_its_commit_and_branch() {
    let gh = ScriptedGh::new(vec![Ok(GhOutput {
        code: Some(0),
        stdout: r#"{"headRefOid":"ABCD1234","headRefName":"feature/x"}"#.into(),
        stderr: String::new(),
    })]);
    let head = super::head(&gh, "https://github.com/o/r/pull/7").unwrap();
    assert_eq!(
        (head.commit.as_str(), head.branch.as_str()),
        ("abcd1234", "feature/x")
    );
    assert!(gh.calls()[0].contains("headRefOid,headRefName"));

    let az = ScriptedGh::new(vec![Ok(GhOutput {
        code: Some(0),
        stdout: r#"{"pullRequestId":535,"repository":{"name":"Repo","project":{"name":"Proj"}},"sourceRefName":"refs/heads/feature/x","lastMergeSourceCommit":{"commitId":"abcd1234"}}"#.into(),
        stderr: String::new(),
    })]);
    let head = super::head(&az, AZURE_PR).unwrap();
    assert_eq!(
        (head.commit.as_str(), head.branch.as_str()),
        ("abcd1234", "feature/x")
    );
}

#[test]
fn a_head_without_a_branch_is_an_error() {
    let gh = ScriptedGh::new(vec![Ok(GhOutput {
        code: Some(0),
        stdout: r#"{"headRefOid":"abcd1234"}"#.into(),
        stderr: String::new(),
    })]);
    assert!(super::head(&gh, "https://github.com/o/r/pull/7").is_err());
}
