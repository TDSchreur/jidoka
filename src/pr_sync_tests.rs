//! `jidoka pr sync` with real git in temporary repositories and a local bare remote.

use super::*;
use std::{cell::RefCell, fs, path::PathBuf};
use tempfile::TempDir;

/// The plain path form git prints; on Windows `canonicalize` gives a `\\?\` path that git reads as a URL.
fn real_path(path: &Path) -> PathBuf {
    let real = path.canonicalize().unwrap();
    match real.to_string_lossy().strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => real,
    }
}

/// Git without the user's own settings, with a fixed author so that merges can be committed.
fn isolated(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("jidoka-tests-no-such-gitconfig"),
        )
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
}

fn run_git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false"])
        .args(args);
    let output = isolated(&mut command).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The code's git, isolated like the tests' own; `fail_merge_base` makes `merge-base --is-ancestor` fail oddly.
#[derive(Default)]
struct TestGit {
    fail_merge_base: bool,
    /// Plays a git older than 2.38, which has no `merge-tree --write-tree`.
    old_git: bool,
    /// Every git command the code ran, so a test can see what it never does.
    commands: RefCell<Vec<Vec<String>>>,
}

impl TestGit {
    fn record(&self, args: &[&str]) {
        self.commands
            .borrow_mut()
            .push(args.iter().map(|a| a.to_string()).collect());
    }
}

impl Git for TestGit {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        self.record(args);
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(dir)
            .args(["-c", "commit.gpgsign=false"])
            .args(args);
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

    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        self.record(args);
        if self.old_git && args.first() == Some(&"merge-tree") {
            return Ok(129);
        }
        // Only the behind test: is the base in HEAD?
        if self.fail_merge_base
            && args.first() == Some(&"merge-base")
            && args.get(2) == Some(&"refs/remotes/origin/develop")
        {
            return Ok(128);
        }
        let mut command = Command::new("git");
        command.arg("-C").arg(dir).args(args);
        let output = isolated(&mut command).output().map_err(|e| e.to_string())?;
        output.status.code().ok_or_else(|| "signal".into())
    }
}

/// A bare remote with `develop` (a.txt of ten lines, b.txt), a clone `repo`, a worktree `wt` on `feature`
/// that changed line 1 of a.txt and was pushed, and a second clone `other` that plays the teammate.
struct Project {
    _dir: TempDir,
    remote: PathBuf,
    wt: PathBuf,
    other: PathBuf,
}

const TEN_LINES: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";

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
        fs::write(seed.join("a.txt"), TEN_LINES).unwrap();
        fs::write(seed.join("b.txt"), "b\n").unwrap();
        run_git(&seed, &["add", "."]);
        run_git(&seed, &["commit", "-q", "-m", "first"]);
        run_git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run_git(&seed, &["push", "-q", "origin", "develop"]);
        let clone = |name: &str| {
            let path = root.join(name);
            let mut command = Command::new("git");
            command.args(["clone", "-q"]).arg(&remote).arg(&path);
            assert!(isolated(&mut command).output().unwrap().status.success());
            path
        };
        let repo = clone("repo");
        let other = clone("other");
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
        fs::write(wt.join("a.txt"), TEN_LINES.replacen("1\n", "one\n", 1)).unwrap();
        run_git(&wt, &["commit", "-q", "-am", "feature work"]);
        run_git(&wt, &["push", "-q", "-u", "origin", "feature"]);
        Self {
            _dir: dir,
            remote,
            wt,
            other,
        }
    }

    /// The teammate changes the base: `file` gets `content`.
    fn base_moves(&self, file: &str, content: &str) {
        run_git(&self.other, &["pull", "-q", "origin", "develop"]);
        fs::write(self.other.join(file), content).unwrap();
        run_git(&self.other, &["commit", "-q", "-am", "teammate"]);
        run_git(&self.other, &["push", "-q", "origin", "develop"]);
    }

    fn remote_branch(&self, branch: &str) -> String {
        run_git(
            &self.remote,
            &["rev-parse", &format!("refs/heads/{branch}")],
        )
    }

    fn head(&self) -> String {
        run_git(&self.wt, &["rev-parse", "HEAD"])
    }
}

/// One call of `RunApi::update`: run, stage, status and options.
type Update = (String, String, String, Vec<String>);

#[derive(Default)]
struct FakeApi {
    updates: RefCell<Vec<Update>>,
}

impl RunApi for FakeApi {
    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String> {
        self.updates.borrow_mut().push((
            id.into(),
            stage.into(),
            status.into(),
            options.iter().map(|o| o.to_string()).collect(),
        ));
        Ok(())
    }
}

fn run_of(project: &Project) -> PrRun {
    PrRun {
        id: "100".into(),
        worktree: Some(project.wt.display().to_string()),
        branch: Some("feature".into()),
        stage: "finish".into(),
        status: "running".into(),
    }
}

fn options() -> Options {
    Options {
        base: Some("develop".into()),
        json: false,
    }
}

fn sync_now(project: &Project, api: &FakeApi) -> Result<Synced, ToolProblem> {
    sync(&run_of(project), &options(), &TestGit::default(), api)
}

#[test]
fn a_clean_merge_that_touches_none_of_the_change_is_pushed() {
    let project = Project::new();
    project.base_moves("b.txt", "b changed\n");
    let api = FakeApi::default();
    let synced = sync_now(&project, &api).unwrap();
    assert_eq!((synced.outcome, synced.exit_code), (Outcome::Pushed, 2));
    // A normal merge commit with the base as its second parent, pushed as it is.
    let merge = synced.merge_commit.clone().unwrap();
    assert_eq!(project.head(), merge);
    assert_eq!(project.remote_branch("feature"), merge);
    let parents = run_git(&project.wt, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 3, "{parents}");
    assert_eq!(
        run_git(&project.wt, &["log", "-1", "--format=%s"]),
        "Merge origin/develop into feature"
    );
    assert!(render(&synced).contains("jidoka pr wait"));
    let updates = api.updates.borrow();
    let (_, stage, status, args) = updates.last().unwrap();
    assert_eq!((stage.as_str(), status.as_str()), ("finish", "running"));
    assert!(
        args.iter().any(|a| a == "Merged develop into the branch"),
        "{args:?}"
    );
}

#[test]
fn a_clean_merge_that_touches_the_change_is_kept_for_verification() {
    let project = Project::new();
    // The teammate changes the last line of the file the change also changed: no conflict, but overlap.
    project.base_moves("a.txt", &TEN_LINES.replace("10\n", "ten\n"));
    let pushed_before = project.remote_branch("feature");
    let synced = sync_now(&project, &FakeApi::default()).unwrap();
    assert_eq!(
        (synced.outcome, synced.exit_code),
        (Outcome::NeedsVerification, 3)
    );
    assert_eq!(synced.files, ["a.txt"]);
    assert!(synced.merge_commit.is_some());
    assert_eq!(
        project.remote_branch("feature"),
        pushed_before,
        "not pushed"
    );
    assert_eq!(
        fs::read_to_string(project.wt.join("a.txt")).unwrap(),
        TEN_LINES
            .replacen("1\n", "one\n", 1)
            .replace("10\n", "ten\n")
    );
}

#[test]
fn a_conflict_is_left_in_progress_with_its_files_and_a_second_run_refuses() {
    let project = Project::new();
    project.base_moves("a.txt", &TEN_LINES.replacen("1\n", "uno\n", 1));
    let synced = sync_now(&project, &FakeApi::default()).unwrap();
    assert_eq!((synced.outcome, synced.exit_code), (Outcome::Conflict, 4));
    assert_eq!(synced.files, ["a.txt"]);
    // The merge is still in progress for the agent to resolve.
    run_git(&project.wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    let problem = sync_now(&project, &FakeApi::default()).unwrap_err();
    assert!(problem.0.contains("merge is in progress"), "{}", problem.0);
}

#[test]
fn a_failed_fetch_changes_nothing() {
    let project = Project::new();
    run_git(
        &project.wt,
        &[
            "remote",
            "set-url",
            "origin",
            project.remote.join("missing").to_str().unwrap(),
        ],
    );
    let head = project.head();
    let problem = sync_now(&project, &FakeApi::default()).unwrap_err();
    assert!(problem.0.to_lowercase().contains("fetch"), "{}", problem.0);
    assert_eq!(project.head(), head);
}

#[test]
fn a_push_that_failed_is_finished_by_running_the_command_again() {
    let project = Project::new();
    project.base_moves("b.txt", "b changed\n");
    // The remote refuses every push for now.
    let hook = project.remote.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\necho 'not now' >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let before = project.remote_branch("feature");
    let problem = sync_now(&project, &FakeApi::default()).unwrap_err();
    assert!(problem.0.contains("again"), "{}", problem.0);
    // A hook that refuses the push is not a remote branch that moved on: git's reason is shown.
    assert!(!problem.0.contains("moved on"), "{}", problem.0);
    assert!(problem.0.contains("declined"), "{}", problem.0);
    assert_eq!(project.remote_branch("feature"), before);
    let merge = project.head();
    assert_ne!(merge, before, "the merge commit is kept");

    fs::remove_file(&hook).unwrap();
    let synced = sync_now(&project, &FakeApi::default()).unwrap();
    assert_eq!(synced.outcome, Outcome::Pushed);
    assert_eq!(project.remote_branch("feature"), merge);
}

#[test]
fn the_checks_stop_the_command_before_anything_changes() {
    // Uncommitted changes.
    let project = Project::new();
    project.base_moves("b.txt", "b changed\n");
    fs::write(project.wt.join("b.txt"), "mine\n").unwrap();
    let head = project.head();
    assert!(
        sync_now(&project, &FakeApi::default())
            .unwrap_err()
            .0
            .contains("uncommitted")
    );
    assert_eq!(project.head(), head);

    // The base branch, a wrong branch, a name that is not safe, no worktree, a worktree that is gone.
    let project = Project::new();
    let mut run = run_of(&project);
    run.branch = Some("other".into());
    assert!(
        sync(&run, &options(), &TestGit::default(), &FakeApi::default())
            .unwrap_err()
            .0
            .contains("other")
    );
    let mut base = options();
    base.base = Some("feature".into());
    assert!(
        sync(
            &run_of(&project),
            &base,
            &TestGit::default(),
            &FakeApi::default()
        )
        .unwrap_err()
        .0
        .contains("base branch")
    );
    let mut run = run_of(&project);
    run.worktree = None;
    assert!(
        sync(&run, &options(), &TestGit::default(), &FakeApi::default())
            .unwrap_err()
            .0
            .contains("no worktree")
    );
    run.worktree = Some(project.wt.join("gone").display().to_string());
    assert!(
        sync(&run, &options(), &TestGit::default(), &FakeApi::default())
            .unwrap_err()
            .0
            .contains("does not exist")
    );
    run_git(&project.wt, &["branch", "-m", "+feature"]);
    let mut run = run_of(&project);
    run.branch = Some("+feature".into());
    assert!(
        sync(&run, &options(), &TestGit::default(), &FakeApi::default())
            .unwrap_err()
            .0
            .contains("name")
    );
}

// --- rework 1 ---------------------------------------------------------------------------------------

#[test]
fn running_again_never_pushes_a_merge_that_still_needs_verification() {
    let project = Project::new();
    project.base_moves("a.txt", &TEN_LINES.replace("10\n", "ten\n"));
    let before = project.remote_branch("feature");
    assert_eq!(
        sync_now(&project, &FakeApi::default()).unwrap().outcome,
        Outcome::NeedsVerification
    );
    let again = sync_now(&project, &FakeApi::default()).unwrap();
    assert_eq!(again.outcome, Outcome::Current);
    assert_eq!(again.unpushed, 1, "the merge is reported, not pushed");
    assert_eq!(project.remote_branch("feature"), before);
}

// --- rework 2 ---------------------------------------------------------------------------------------
