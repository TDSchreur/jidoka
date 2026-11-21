//! `jidoka pr open` with real git in temporary repositories (a local bare remote) and a fake host.

use super::*;
use crate::pr_host::{FakeHost, Host, OpenPr, Origin};
use std::{cell::RefCell, fs, path::Path, process::Command};
use tempfile::TempDir;

/// The plain path form git prints; on Windows `canonicalize` gives a `\\?\` path that git reads as a URL.
fn real_path(path: &Path) -> PathBuf {
    let real = path.canonicalize().unwrap();
    match real.to_string_lossy().strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => real,
    }
}

fn isolated(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "safe.bareRepository")
        .env("GIT_CONFIG_VALUE_0", "all")
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("jidoka-tests-no-such-gitconfig"),
        )
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
}

fn run_git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args);
    let output = isolated(&mut command).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

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
        let mut clone = Command::new("git");
        clone.args(["clone", "-q"]).arg(&remote).arg(&repo);
        assert!(isolated(&mut clone).output().unwrap().status.success());
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
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.remote).args([
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{branch}"),
        ]);
        let output = isolated(&mut command).output().unwrap();
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
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
        let mut command = Command::new("git");
        command.arg("-C").arg(dir).args(args).env("LC_ALL", "C");
        let output = isolated(&mut command).output().map_err(|e| e.to_string())?;
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
        if output.status.success() {
            Ok(text(&output.stdout))
        } else if output.stderr.is_empty() {
            Err(text(&output.stdout))
        } else {
            Err(text(&output.stderr))
        }
    }
}

/// How many calls of `host` start with `what`.
fn host_calls(host: &FakeHost, what: &str) -> usize {
    host.calls().iter().filter(|c| c.starts_with(what)).count()
}

/// One call of `RunApi::update`: run and options.
type Update = (String, Vec<String>);

#[derive(Default)]
struct FakeApi {
    updates: RefCell<Vec<Update>>,
    fail: std::cell::Cell<bool>,
}

impl RunApi for FakeApi {
    fn update(&self, id: &str, options: &[&str]) -> Result<(), String> {
        if self.fail.get() {
            return Err("the server is not reachable".into());
        }
        self.updates
            .borrow_mut()
            .push((id.into(), options.iter().map(|o| o.to_string()).collect()));
        Ok(())
    }
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
    let updates = api.updates.borrow();
    let (id, args) = updates.last().unwrap();
    assert_eq!(id, "100");
    assert_eq!(args[..2], ["--pr".to_string(), URL.to_string()]);
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
    assert_eq!(api.updates.borrow().len(), 1);
}

#[test]
fn nothing_is_pushed_when_a_check_fails_first() {
    // Uncommitted changes, one of the Workspace checks `pr sync` shares (tested in pr_sync).
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
    let mut clone = Command::new("git");
    clone.args(["clone", "-q"]).arg(&project.remote).arg(&other);
    assert!(isolated(&mut clone).output().unwrap().status.success());
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
        problem.0.contains(URL) && problem.0.contains("run could not be updated"),
        "{}",
        problem.0
    );

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
