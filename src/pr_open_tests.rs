//! `factory pr open` with real git in temporary repositories (a local bare remote) and a scripted `gh`.

use super::*;
use std::{cell::RefCell, collections::VecDeque, fs};
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
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("factory-tests-no-such-gitconfig"),
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

/// A bare remote with `develop`, a clone `repo`, and a worktree `wt` on a branch `feature` with one commit.
struct Project {
    _dir: TempDir,
    remote: PathBuf,
    repo: PathBuf,
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
            repo,
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

#[derive(Default)]
struct FakeGh {
    version: RefCell<Option<Result<GhOutput, GhError>>>,
    list: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    create: RefCell<VecDeque<Result<GhOutput, GhError>>>,
    calls: RefCell<Vec<(PathBuf, String)>>,
}

fn ok(stdout: &str) -> Result<GhOutput, GhError> {
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

fn pop(queue: &RefCell<VecDeque<Result<GhOutput, GhError>>>) -> Result<GhOutput, GhError> {
    queue
        .borrow_mut()
        .pop_front()
        .expect("the script has an answer")
}

impl Gh for FakeGh {
    /// The code under test runs git; the user's own git settings must not reach it.
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
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

    fn run(&self, dir: &Path, args: &[&str]) -> Result<GhOutput, GhError> {
        self.calls
            .borrow_mut()
            .push((dir.to_path_buf(), args.join(" ")));
        match args {
            ["--version"] => match self.version.borrow().as_ref() {
                Some(Err(GhError::NotInstalled)) => Err(GhError::NotInstalled),
                _ => ok("gh version 2"),
            },
            ["pr", "list", ..] => pop(&self.list),
            ["pr", "create", ..] => pop(&self.create),
            other => panic!("unexpected gh call {other:?}"),
        }
    }
}

impl FakeGh {
    /// No open pull request yet, and creating one answers with `url`.
    fn new_pr(url: &str) -> Self {
        let gh = Self::default();
        gh.list.borrow_mut().push_back(ok("[]"));
        gh.create.borrow_mut().push_back(ok(&format!(
            "Creating pull request for feature into develop\n\n{url}\n"
        )));
        gh
    }

    fn pr_calls(&self, what: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|(_, c)| c.starts_with(what))
            .count()
    }
}

/// One call of `RunApi::update`: run, stage, status and options.
type Update = (String, String, String, Vec<String>);

#[derive(Default)]
struct FakeApi {
    updates: RefCell<Vec<Update>>,
    fail: std::cell::Cell<bool>,
}

impl RunApi for FakeApi {
    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String> {
        if self.fail.get() {
            return Err("the server is not reachable".into());
        }
        self.updates.borrow_mut().push((
            id.into(),
            stage.into(),
            status.into(),
            options.iter().map(|o| o.to_string()).collect(),
        ));
        Ok(())
    }
}

fn run_of(project: &Project) -> RunInfo {
    RunInfo {
        id: "100".into(),
        worktree: Some(project.wt.display().to_string()),
        branch: Some("feature".into()),
        stage: "finish".into(),
        status: "running".into(),
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
    let gh = FakeGh::new_pr(URL);
    let api = FakeApi::default();
    let opened = open(&run_of(&project), &options(&project), &gh, &api).unwrap();

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
    // The pull request was asked for in the worktree, against the base, with the title and body file.
    let calls = gh.calls.borrow();
    let (dir, create) = calls
        .iter()
        .find(|(_, c)| c.starts_with("pr create"))
        .unwrap();
    assert_eq!(dir, &project.wt);
    assert!(
        create.contains("--base develop --head feature --title Add the thing --body-file"),
        "{create}"
    );
    // The run records it, keeping its stage.
    let updates = api.updates.borrow();
    let (id, stage, status, args) = updates.last().unwrap();
    assert_eq!(
        (id.as_str(), stage.as_str(), status.as_str()),
        ("100", "finish", "running")
    );
    assert_eq!(args[..2], ["--pr".to_string(), URL.to_string()]);
}

#[test]
fn a_retry_finds_the_open_pull_request_and_does_not_make_a_second() {
    let project = Project::new();
    let gh = FakeGh::default();
    gh.list.borrow_mut().push_back(ok(&format!(
        r#"[{{"url":"{URL}","baseRefName":"release"}}]"#
    )));
    let api = FakeApi::default();
    let opened = open(&run_of(&project), &options(&project), &gh, &api).unwrap();
    assert_eq!(opened.pull_request, URL);
    assert!(!opened.created);
    assert_eq!(opened.base, "release", "the base of the open pull request");
    assert_eq!(gh.pr_calls("pr create"), 0);
    // New commits still reach the pull request, and the run records it again.
    assert!(project.remote_branch("feature").is_some());
    assert_eq!(api.updates.borrow().len(), 1);
}

#[test]
fn nothing_is_pushed_when_a_check_fails_first() {
    // Uncommitted changes.
    let project = Project::new();
    fs::write(project.wt.join("work.txt"), "unsaved").unwrap();
    let problem = open(
        &run_of(&project),
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("uncommitted"), "{}", problem.0);
    assert_eq!(project.remote_branch("feature"), None);

    // The branch is the base.
    let project = Project::new();
    let mut run = run_of(&project);
    run.worktree = Some(project.repo.display().to_string());
    run.branch = Some("develop".into());
    let problem = open(
        &run,
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("base branch"), "{}", problem.0);

    // No commits beyond the base.
    let project = Project::new();
    run_git(&project.wt, &["reset", "-q", "--hard", "origin/develop"]);
    let problem = open(
        &run_of(&project),
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("no commits beyond"), "{}", problem.0);
    assert_eq!(project.remote_branch("feature"), None);

    // The worktree is on another branch than the run records.
    let project = Project::new();
    let mut run = run_of(&project);
    run.branch = Some("other".into());
    let problem = open(
        &run,
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(
        problem.0.contains("other") && problem.0.contains("feature"),
        "{}",
        problem.0
    );

    // The run records no worktree, or one that is gone.
    let project = Project::new();
    let mut run = run_of(&project);
    run.worktree = None;
    assert!(open(
        &run,
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default()
    )
    .unwrap_err()
    .0
    .contains("no worktree"));
    run.worktree = Some(project.wt.join("gone").display().to_string());
    assert!(open(
        &run,
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default()
    )
    .unwrap_err()
    .0
    .contains("does not exist"));
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

    let gh = FakeGh::new_pr(URL);
    let problem = open(
        &run_of(&project),
        &options(&project),
        &gh,
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
    assert_eq!(gh.pr_calls("pr create"), 0);
}

#[test]
fn a_missing_gh_or_a_missing_sign_in_stops_before_anything_is_pushed() {
    let project = Project::new();
    let gh = FakeGh::default();
    *gh.version.borrow_mut() = Some(Err(GhError::NotInstalled));
    let problem = open(
        &run_of(&project),
        &options(&project),
        &gh,
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(
        problem.0.contains("GitHub CLI") && problem.0.contains("cli.github.com"),
        "{}",
        problem.0
    );
    assert_eq!(project.remote_branch("feature"), None);

    let gh = FakeGh::default();
    gh.list.borrow_mut().push_back(failed(
        4,
        "To get started with GitHub CLI, please run:  gh auth login",
    ));
    let problem = open(
        &run_of(&project),
        &options(&project),
        &gh,
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("gh auth login"), "{}", problem.0);
    assert_eq!(
        project.remote_branch("feature"),
        None,
        "signed out: nothing was pushed"
    );
}

#[test]
fn a_failed_creation_or_a_failed_record_says_what_happened_and_a_retry_is_safe() {
    let project = Project::new();
    let gh = FakeGh::default();
    // Nothing is open before the creation, and still nothing is open after it failed.
    gh.list.borrow_mut().push_back(ok("[]"));
    gh.list.borrow_mut().push_back(ok("[]"));
    gh.create
        .borrow_mut()
        .push_back(failed(1, "GraphQL: Something went wrong"));
    let problem = open(
        &run_of(&project),
        &options(&project),
        &gh,
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("Something went wrong"), "{}", problem.0);

    // The pull request opened but the server did not take the record: the message names the URL.
    let api = FakeApi::default();
    api.fail.set(true);
    let problem = open(
        &run_of(&project),
        &options(&project),
        &FakeGh::new_pr(URL),
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
    open(&run, &options(&project), &FakeGh::new_pr(URL), &api).unwrap();
    assert!(api.updates.borrow().is_empty());
}

#[test]
fn the_url_is_the_last_url_gh_printed() {
    let project = Project::new();
    let gh = FakeGh::default();
    gh.list.borrow_mut().push_back(ok("[]"));
    gh.create.borrow_mut().push_back(ok(
        "Warning: 1 uncommitted change\nhttps://github.com/o/r/pull/9\n",
    ));
    assert_eq!(
        open(
            &run_of(&project),
            &options(&project),
            &gh,
            &FakeApi::default()
        )
        .unwrap()
        .pull_request,
        "https://github.com/o/r/pull/9"
    );
    let gh = FakeGh::default();
    gh.list.borrow_mut().push_back(ok("[]"));
    gh.create.borrow_mut().push_back(ok("no url here\n"));
    assert!(open(
        &run_of(&project),
        &options(&project),
        &gh,
        &FakeApi::default()
    )
    .unwrap_err()
    .0
    .contains("URL"));
}

#[test]
fn the_options_are_checked_before_anything_runs() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("body.md");
    fs::write(&body, "x").unwrap();
    let b = body.to_str().unwrap();
    let ok =
        parse_options(&["--title", "T", "--body-file", b, "--base", "main", "--json"]).unwrap();
    assert!(ok.json && ok.base.as_deref() == Some("main") && ok.title == "T");
    assert!(ok.body_file.is_absolute());
    for bad in [
        vec![],
        vec!["--title", "T"],
        vec!["--body-file", b],
        vec!["--title", "", "--body-file", b],
        vec!["--title", "a\nb", "--body-file", b],
        vec!["--title", "T", "--body-file", "/no/such/file.md"],
        vec!["--title", "T", "--body-file", b, "--base", "-x"],
        vec!["--title", "T", "--body-file", b, "--title", "U"],
        vec!["--title", "T", "--body-file", b, "--json", "--json"],
        vec!["--title", "T", "--body-file", b, "--bogus"],
        vec!["--title", "T", "--body-file"],
    ] {
        assert!(parse_options(&bad).is_err(), "{bad:?}");
    }
    let long = "x".repeat(300);
    assert!(parse_options(&["--title", &long, "--body-file", b]).is_err());
    // A folder is not a body file.
    assert!(parse_options(&[
        "--title",
        "T",
        "--body-file",
        std::env::temp_dir().to_str().unwrap()
    ])
    .is_err());
}

#[test]
fn the_text_and_the_json_say_the_same_and_are_printable() {
    let opened = Opened {
        pull_request: format!("{URL}\u{1b}[2J"),
        created: true,
        branch: "feature".into(),
        base: "develop".into(),
        commit: "0123456789".into(),
        recorded: true,
    };
    let text = render(&opened, &run_of(&Project::new()));
    assert!(text.starts_with("Pull request opened: "), "{text}");
    assert!(text.contains("0123456") && !text.contains("0123456789"));
    assert!(!text.chars().any(|c| c.is_control() && c != '\n'));
    let json = serde_json::to_value(&opened).unwrap();
    assert_eq!(json["created"], true);
    assert_eq!(json["base"], "develop");
    // Every field of the data is in the text (the commit as its first seven characters).
    for field in ["branch", "base"] {
        assert!(
            text.contains(json[field].as_str().unwrap()),
            "{field}: {text}"
        );
    }
    assert!(text.contains(&json["commit"].as_str().unwrap()[..7]));
    let again = Opened {
        created: false,
        ..opened
    };
    assert!(render(&again, &run_of(&Project::new())).starts_with("Pull request already open: "));
}

#[test]
fn nothing_in_the_command_forces_a_push_or_changes_the_work() {
    let source = include_str!("pr_open.rs");
    for forbidden in [
        "\"--force\"",
        "\"-f\"",
        "\"--force-with-lease\"",
        "\"+",
        "\"reset\"",
        "\"checkout\"",
        "\"clean\"",
        "\"commit\"",
        "\"add\"",
        "\"fetch\"",
        "\"merge\"",
    ] {
        assert!(!source.contains(forbidden), "{forbidden}");
    }
}

// --- rework 1 ---------------------------------------------------------------------------------------

#[test]
fn a_branch_that_git_would_read_as_a_forced_refspec_is_never_pushed() {
    let project = Project::new();
    // The remote's develop has a commit that the local develop does not have.
    let other = project.wt.parent().unwrap().join("other");
    let mut clone = Command::new("git");
    clone.args(["clone", "-q"]).arg(&project.remote).arg(&other);
    assert!(isolated(&mut clone).output().unwrap().status.success());
    run_git(&other, &["commit", "-q", "--allow-empty", "-m", "teammate"]);
    run_git(&other, &["push", "-q", "origin", "develop"]);
    let theirs = project.remote_branch("develop").unwrap();

    // A worktree on `+develop`: git reads `git push origin +develop` as a forced update of develop.
    let plus = project.wt.parent().unwrap().join("plus");
    run_git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "+develop",
            plus.to_str().unwrap(),
            "origin/develop",
        ],
    );
    run_git(&plus, &["commit", "-q", "--allow-empty", "-m", "mine"]);
    let mut run = run_of(&project);
    run.worktree = Some(plus.display().to_string());
    run.branch = Some("+develop".into());
    let problem = open(
        &run,
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(
        problem.0.contains("+develop") && problem.0.contains("name"),
        "{}",
        problem.0
    );
    assert_eq!(
        project.remote_branch("develop"),
        Some(theirs),
        "the teammate's commit is still there"
    );
}

#[test]
fn the_branch_is_pushed_by_its_full_name_on_both_sides() {
    assert!(include_str!("pr_open.rs").contains("refs/heads/{branch}:refs/heads/{branch}"));
    assert!(!crate::run_begin::valid_branch_name("+main"));
}

#[test]
fn a_pull_request_from_a_fork_is_not_ours() {
    let project = Project::new();
    let gh = FakeGh::default();
    gh.list.borrow_mut().push_back(ok(
        r#"[{"url":"https://github.com/fork/r/pull/3","baseRefName":"develop","isCrossRepository":true}]"#,
    ));
    gh.create.borrow_mut().push_back(ok(&format!("{URL}\n")));
    let opened = open(
        &run_of(&project),
        &options(&project),
        &gh,
        &FakeApi::default(),
    )
    .unwrap();
    assert!(opened.created);
    assert_eq!(opened.pull_request, URL);
}

#[test]
fn losing_the_race_to_create_the_pull_request_finds_the_one_that_won() {
    let project = Project::new();
    let gh = FakeGh::default();
    gh.list.borrow_mut().push_back(ok("[]"));
    gh.list.borrow_mut().push_back(ok(&format!(
        r#"[{{"url":"{URL}","baseRefName":"develop"}}]"#
    )));
    gh.create.borrow_mut().push_back(failed(
        1,
        &format!(
            "a pull request for branch \"feature\" into branch \"develop\" already exists:\n{URL}"
        ),
    ));
    let api = FakeApi::default();
    let opened = open(&run_of(&project), &options(&project), &gh, &api).unwrap();
    assert_eq!((opened.pull_request.as_str(), opened.created), (URL, false));
    assert_eq!(api.updates.borrow().len(), 1);
}

#[test]
fn an_entry_without_a_url_is_an_error_and_not_a_reason_to_open_a_second_pull_request() {
    let project = Project::new();
    let gh = FakeGh::default();
    gh.list
        .borrow_mut()
        .push_back(ok(r#"[{"baseRefName":"develop"}]"#));
    let problem = open(
        &run_of(&project),
        &options(&project),
        &gh,
        &FakeApi::default(),
    )
    .unwrap_err();
    assert!(problem.0.contains("URL"), "{}", problem.0);
    assert_eq!(gh.pr_calls("pr create"), 0);
}

#[test]
fn a_run_that_could_not_be_updated_is_told_how_to_record_the_pull_request() {
    let project = Project::new();
    let mut run = run_of(&project);
    run.status = "done".into();
    let opened = open(
        &run,
        &options(&project),
        &FakeGh::new_pr(URL),
        &FakeApi::default(),
    )
    .unwrap();
    assert!(!opened.recorded);
    let text = render(&opened, &run);
    assert!(text.contains("not updated"), "{text}");
    // The command to run is complete: the run, its stage and status, and the pull request.
    assert!(
        text.contains(&format!("factory run update 100 finish done --pr {URL}")),
        "{text}"
    );
    assert_eq!(serde_json::to_value(&opened).unwrap()["recorded"], false);
}
