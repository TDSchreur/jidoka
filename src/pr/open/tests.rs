//! `jidoka pr open` with real git in temporary repositories (a local bare remote) and a fake host.

use super::*;
use crate::git::fixture::{self, real_path, run_git};
use crate::pr::host::{FakeHost, Host, OpenPr, Origin};
use std::{cell::RefCell, fs, path::Path};
use tempfile::TempDir;

/// A bare remote with `develop`, a clone `repo` (not kept), and a worktree `wt` on a branch `feature` with one commit.
struct Project {
    _dir: TempDir,
    remote: PathBuf,
    wt: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = real_path(dir.path());
        let remote = root.join("remote.git");
        fs::create_dir(&remote).unwrap();
        run_git(&remote, &["init", "-q", "--bare", "-b", "develop"]);
        let seed = root.join("seed");
        fs::create_dir(&seed).unwrap();
        run_git(&seed, &["init", "-q", "-b", "develop"]);
        run_git(&seed, &["commit", "-q", "--allow-empty", "-m", "first"]);
        run_git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run_git(&seed, &["push", "-q", "origin", "develop"]);
        let repo = root.join("repo");
        run_git(
            &root,
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        );
        let wt = root.join("wt");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
                "origin/develop",
            ],
        );
        run_git(&wt, &["commit", "-q", "--allow-empty", "-m", "work"]);
        Self {
            _dir: dir,
            remote,
            wt,
        }
    }

    fn remote_branch(&self, branch: &str) -> Option<String> {
        fixture::git(
            &self.remote,
            &[
                "rev-parse",
                "--verify",
                "-q",
                &format!("refs/heads/{branch}"),
            ],
        )
        .ok()
    }
}

/// Real git with the user's own settings kept out, and origin's URLs answered as set.
#[derive(Default)]
struct FakeGit {
    remote_url: Option<String>,
}

impl Git for FakeGit {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        if args == ["remote", "get-url", "origin"]
            || args == ["remote", "get-url", "--push", "--all", "origin"]
        {
            return Ok(self
                .remote_url
                .clone()
                .unwrap_or_else(|| "https://github.com/o/r.git".into()));
        }
        fixture::git(dir, args)
    }
}

/// How many calls of `host` start with `what`.
fn host_calls(host: &FakeHost, what: &str) -> usize {
    host.calls().iter().filter(|c| c.starts_with(what)).count()
}

/// The reports sent to the run, with the run they were for; `fail` makes the server unreachable.
#[derive(Default)]
struct FakeApi {
    updates: RefCell<Vec<(String, PrReport)>>,
    fail: std::cell::Cell<bool>,
}

impl RunReporter for FakeApi {
    fn report(&self, id: &str, report: &PrReport) -> Result<(), String> {
        if self.fail.get() {
            return Err("the server is not reachable".into());
        }
        self.updates.borrow_mut().push((id.into(), report.clone()));
        Ok(())
    }
}

fn opened_report() -> (String, PrReport) {
    (
        "100".to_string(),
        PrReport::Opened {
            pull_request: URL.into(),
        },
    )
}

fn run_of(project: &Project) -> PrRun {
    PrRun {
        id: "100".into(),
        worktree: Some(project.wt.display().to_string()),
        branch: Some("feature".into()),
        status: "running".into(),
        base_commit: None,
        pr_state: None,
    }
}

fn options(project: &Project) -> Options {
    let body = project.wt.parent().unwrap().join("body.md");
    fs::write(&body, "Body text").unwrap();
    Options {
        title: "Add the thing".into(),
        body_file: body,
        base: Some("develop".into()),
        json: false,
    }
}

const URL: &str = "https://github.com/o/r/pull/7";

#[test]
fn a_new_pull_request_is_pushed_opened_and_recorded() {
    let project = Project::new();
    let host = FakeHost::opening(URL);
    let api = FakeApi::default();
    let opened = open(
        &run_of(&project),
        &options(&project),
        &host,
        &FakeGit::default(),
        &api,
    )
    .unwrap();

    assert_eq!(opened.pull_request, URL);
    assert!(opened.created);
    assert_eq!(
        (opened.branch.as_str(), opened.base.as_str()),
        ("feature", "develop")
    );
    // The branch is on the remote, at the commit that was reported.
    assert_eq!(
        project.remote_branch("feature"),
        Some(opened.commit.clone())
    );
    // The host was asked for the pull request against the base, with the title.
    assert_eq!(
        host.calls(),
        [
            "find_open feature",
            "check_new feature",
            "create feature into develop: Add the thing"
        ]
    );
    // The run records it, keeping its stage.
    assert_eq!(*api.updates.borrow(), [opened_report()]);
}

#[test]
fn a_retry_finds_the_open_pull_request_and_does_not_make_a_second() {
    let project = Project::new();
    let host = FakeHost::with_open(OpenPr {
        url: URL.into(),
        base: Some("release".into()),
    });
    let api = FakeApi::default();
    let opened = open(
        &run_of(&project),
        &options(&project),
        &host,
        &FakeGit::default(),
        &api,
    )
    .unwrap();
    assert_eq!(opened.pull_request, URL);
    assert!(!opened.created);
    assert_eq!(opened.base, "release", "the base of the open pull request");
    assert_eq!(host_calls(&host, "create"), 0);
    // New commits still reach the pull request, and the run records it again.
    assert!(project.remote_branch("feature").is_some());
    assert_eq!(*api.updates.borrow(), [opened_report()]);
}

#[test]
fn nothing_is_pushed_when_a_check_fails_first() {
    // Uncommitted changes, one of the Workspace checks `pr sync` shares (tested in `pr::sync`).
    let project = Project::new();
    fs::write(project.wt.join("work.txt"), "unsaved").unwrap();
    let problem = open(
        &run_of(&project),
        &options(&project),
        &FakeHost::opening(URL),
        &FakeGit::default(),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("uncommitted"), "{}", problem.0);
    assert_eq!(project.remote_branch("feature"), None);

    // No commits beyond the base.
    let project = Project::new();
    run_git(&project.wt, &["reset", "-q", "--hard", "origin/develop"]);
    let problem = open(
        &run_of(&project),
        &options(&project),
        &FakeHost::opening(URL),
        &FakeGit::default(),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("no commits beyond"), "{}", problem.0);
    assert_eq!(project.remote_branch("feature"), None);
}

#[test]
fn a_rejected_push_says_to_merge_and_never_forces() {
    let project = Project::new();
    // The remote already has a different commit on the branch.
    let other = project.wt.parent().unwrap().join("other");
    run_git(
        project.wt.parent().unwrap(),
        &[
            "clone",
            "-q",
            project.remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    run_git(
        &other,
        &["checkout", "-q", "-b", "feature", "origin/develop"],
    );
    run_git(
        &other,
        &["commit", "-q", "--allow-empty", "-m", "someone else"],
    );
    run_git(&other, &["push", "-q", "origin", "feature"]);
    let theirs = project.remote_branch("feature").unwrap();

    let host = FakeHost::opening(URL);
    let problem = open(
        &run_of(&project),
        &options(&project),
        &host,
        &FakeGit::default(),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(
        problem.0.contains("rejected") && problem.0.contains("never a force-push"),
        "{}",
        problem.0
    );
    assert_eq!(
        project.remote_branch("feature"),
        Some(theirs),
        "the remote branch is untouched"
    );
    assert_eq!(host_calls(&host, "create"), 0);
}

#[test]
fn a_failed_creation_or_a_failed_record_says_what_happened_and_a_retry_is_safe() {
    let project = Project::new();
    // Nothing is open before the creation, and still nothing is open after it failed.
    let host = FakeHost::failing_create("`gh pr create` failed: GraphQL: Something went wrong");
    let problem = open(
        &run_of(&project),
        &options(&project),
        &host,
        &FakeGit::default(),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("Something went wrong"), "{}", problem.0);
    assert_eq!(host_calls(&host, "find_open"), 2, "it looked again");

    // The pull request opened but the server did not take the record: the message names the URL.
    let api = FakeApi::default();
    api.fail.set(true);
    let problem = open(
        &run_of(&project),
        &options(&project),
        &FakeHost::opening(URL),
        &FakeGit::default(),
        &api,
    )
    .unwrap_err();
    assert!(
        problem.0.contains(URL)
            && problem.0.contains("run could not be updated")
            && problem.0.contains("Run the command again"),
        "{}",
        problem.0
    );
    // The retry it asks for finds that pull request, opens no second one and records it.
    let host = FakeHost::with_open(OpenPr {
        url: URL.into(),
        base: Some("develop".into()),
    });
    let api = FakeApi::default();
    let opened = open(
        &run_of(&project),
        &options(&project),
        &host,
        &FakeGit::default(),
        &api,
    )
    .unwrap();
    assert!(!opened.created && opened.recorded);
    assert_eq!(host_calls(&host, "create"), 0);
    assert_eq!(*api.updates.borrow(), [opened_report()]);

    // A run that is not running is not updated, and the pull request is still reported.
    let mut run = run_of(&project);
    run.status = "done".into();
    let api = FakeApi::default();
    open(
        &run,
        &options(&project),
        &FakeHost::opening(URL),
        &FakeGit::default(),
        &api,
    )
    .unwrap();
    assert!(api.updates.borrow().is_empty());
}

// --- rework 1 ---------------------------------------------------------------------------------------

#[test]
fn an_azure_origin_opens_through_the_host_and_pushes() {
    let project = Project::new();
    let url = "https://dev.azure.com/dutchworkz/Projects/_git/Kahoot/pullrequest/17";
    let git = FakeGit {
        remote_url: Some("https://dev.azure.com/dutchworkz/Projects/_git/Kahoot".into()),
    };
    let opened = open(
        &run_of(&project),
        &options(&project),
        &FakeHost::opening(url),
        &git,
        &FakeApi::default(),
    )
    .unwrap();
    assert_eq!(opened.pull_request, url);
    assert!(opened.created);
    assert_eq!(project.remote_branch("feature"), Some(opened.commit));
}

#[test]
fn a_failed_push_login_shows_the_credential_helper_lines_before_fatal() {
    let error = "You must install or update .NET to run this application.\nfatal: could not read Username for 'https://github.com': terminal prompts disabled\n";
    let origin = Origin {
        url: "https://github.com/o/r.git".into(),
        host: Host::GitHub,
    };
    let problem = push_problem(&origin, "feature", error);
    assert!(
        problem.0.contains("You must install or update .NET"),
        "{}",
        problem.0
    );
    assert!(
        problem.0.contains("terminal prompts disabled"),
        "{}",
        problem.0
    );
    assert!(problem.0.contains("credential.helper"), "{}", problem.0);
}
