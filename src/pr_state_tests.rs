//! Pull request state against a scripted `gh`, and the refresh pass against a real run store.

use super::*;
use crate::pr_wait::{GhError, GhOutput};
use crate::run::{NewRun, RunStore, UpdateRun};
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

impl Gh for ScriptedGh {
    fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        self.calls.lock().unwrap().push(args.join(" "));
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
    // Only http(s) URLs are pull request links; anything else is never handed to a tool.
    let unused = ScriptedGh::new(vec![view("OPEN", false)]);
    assert!(read(&unused, "--help").is_err());
    assert!(unused.calls().is_empty());
}

struct Dir(std::path::PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn store() -> (Dir, RunStore) {
    let dir = std::env::temp_dir().join(format!(
        "factory-pr-state-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = RunStore::open(&dir.join("runs.json")).unwrap();
    (Dir(dir), store)
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
