//! `jidoka run begin` with a fake server and real git in temporary repositories with a local bare remote.

use super::*;
use std::{cell::RefCell, fs, path::PathBuf, process::Command};
use tempfile::TempDir;

/// The real path of `path` in the plain form git prints. On Windows `canonicalize` gives a `\\?\` path, which
/// git reads as a URL when it is used as a remote.
fn real_path(path: &Path) -> PathBuf {
    let real = path.canonicalize().unwrap();
    match real.to_string_lossy().strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => real,
    }
}

/// A user's own git settings (a signing key, hooks) must not reach the tests: this file never exists.
fn no_global_config() -> PathBuf {
    std::env::temp_dir().join("jidoka-tests-no-such-gitconfig")
}

fn isolated(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", no_global_config())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
}

fn git(dir: &Path, args: &[&str]) -> String {
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

/// A bare remote with branches `develop` and `release`, and a clone of it named `repo`.
struct Project {
    _dir: TempDir,
    remote: PathBuf,
    repo: PathBuf,
    worktrees: PathBuf,
}

/// Copies `from` to `to`, creating directories as needed.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Builds the remote and its clone once. Starting git is slow on Windows, so each test copies this instead.
fn template() -> &'static Path {
    static TEMPLATE: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    TEMPLATE
        .get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            let root = real_path(dir.path());
            let remote = root.join("remote.git");
            fs::create_dir(&remote).unwrap();
            git(&remote, &["init", "-q", "--bare", "-b", "develop"]);
            let seed = root.join("seed");
            fs::create_dir(&seed).unwrap();
            git(&seed, &["init", "-q", "-b", "develop"]);
            git(&seed, &["commit", "-q", "--allow-empty", "-m", "first"]);
            git(
                &seed,
                &["remote", "add", "origin", remote.to_str().unwrap()],
            );
            git(&seed, &["push", "-q", "origin", "develop"]);
            git(&seed, &["branch", "release"]);
            git(&seed, &["commit", "-q", "--allow-empty", "-m", "second"]);
            git(&seed, &["push", "-q", "origin", "develop", "release"]);
            fs::remove_dir_all(&seed).unwrap();
            let mut clone_command = Command::new("git");
            clone_command
                .args(["clone", "-q"])
                .arg(&remote)
                .arg(root.join("repo"));
            let clone = isolated(&mut clone_command).output().unwrap();
            assert!(
                clone.status.success(),
                "{}",
                String::from_utf8_lossy(&clone.stderr)
            );
            dir
        })
        .path()
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = real_path(dir.path());
        copy_tree(&real_path(template()), &root);
        let remote = root.join("remote.git");
        let repo = root.join("repo");
        git(
            &repo,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        let worktrees = root.join("repo.worktrees");
        Self {
            _dir: dir,
            remote,
            repo,
            worktrees,
        }
    }

    fn origin_commit(&self, branch: &str) -> String {
        git(
            &self.remote,
            &["rev-parse", &format!("refs/heads/{branch}")],
        )
    }
}

/// One call of `RunApi::update`: run, stage, status and options.
type Update = (String, String, Vec<String>);

#[derive(Default)]
struct FakeApi {
    created: RefCell<Vec<(String, String, Option<String>)>>,
    updates: RefCell<Vec<Update>>,
    /// What `get` answers; `None` means the run is unknown.
    existing: RefCell<Option<RunSummary>>,
    /// Every update fails, as when the server cannot be reached.
    fail_updates: std::cell::Cell<bool>,
    /// Runs that record the session: id and title.
    session_runs: RefCell<Vec<(String, String)>>,
    /// Listing runs fails.
    fail_listing: std::cell::Cell<bool>,
}

impl RunApi for FakeApi {
    fn create(
        &self,
        title: &str,
        project: &str,
        repository: Option<String>,
        _session: Option<String>,
        _routing: Option<ModelRouting>,
    ) -> Result<String, String> {
        self.created
            .borrow_mut()
            .push((title.to_string(), project.to_string(), repository));
        Ok("100".into())
    }

    fn get(&self, id: &str) -> Result<RunSummary, String> {
        self.existing
            .borrow()
            .clone()
            .ok_or_else(|| format!("Run {id} was not found"))
    }

    fn runs_with_session(&self, _session: &str) -> Result<Vec<(String, String)>, String> {
        if self.fail_listing.get() {
            return Err("the server is not reachable".into());
        }
        Ok(self.session_runs.borrow().clone())
    }

    fn update(&self, id: &str, status: &str, options: &[&str]) -> Result<(), String> {
        if self.fail_updates.get() {
            return Err("the server is not reachable".into());
        }
        self.updates.borrow_mut().push((
            id.into(),
            status.into(),
            options.iter().map(|o| o.to_string()).collect(),
        ));
        Ok(())
    }
}

fn options(title: &str) -> Options {
    Options {
        title: title.into(),
        ..Options::default()
    }
}

fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[test]
fn a_fresh_run_gets_a_worktree_from_the_fetched_base_and_records_it() {
    let project = Project::new();
    let api = FakeApi::default();
    let mut opts = options("Add a thing");
    opts.mode = Some("jidoka-full".into());
    let begun = begin(
        &project.repo,
        &opts,
        &api,
        Some(session("sess-1", None)),
        None,
    )
    .unwrap();

    let expected_path = project.worktrees.join("add-a-thing");
    assert_eq!(begun.run, "100");
    assert_eq!(PathBuf::from(&begun.worktree), expected_path);
    assert_eq!(
        (begun.branch.as_str(), begun.base.as_str()),
        ("add-a-thing", "develop")
    );
    assert_eq!(begun.base_commit, project.origin_commit("develop"));
    assert_eq!(
        git(&expected_path, &["branch", "--show-current"]),
        "add-a-thing"
    );
    assert_eq!(
        git(&expected_path, &["rev-parse", "HEAD"]),
        begun.base_commit
    );

    let updates = api.updates.borrow();
    let (id, status, args) = updates.last().unwrap();
    assert_eq!((id.as_str(), status.as_str()), ("100", "running"));
    assert_eq!(value_after(args, "--mode").as_deref(), Some("jidoka-full"));
    assert_eq!(
        value_after(args, "--worktree").as_deref(),
        expected_path.to_str()
    );
    assert_eq!(
        value_after(args, "--branch").as_deref(),
        Some("add-a-thing")
    );
    assert_eq!(
        value_after(args, "--base-commit"),
        Some(begun.base_commit.clone())
    );
    assert_eq!(
        value_after(args, "--activity").as_deref(),
        Some("Workspace ready")
    );
    assert_eq!(value_after(args, "--session").as_deref(), Some("sess-1"));
    // The run was created for the project folder, with the main checkout as its repository.
    let created = api.created.borrow();
    assert_eq!(created[0].0, "Add a thing");
    assert_eq!(created[0].2.as_deref(), project.repo.to_str());
}

fn gitignore_of(worktree: &str) -> Option<String> {
    fs::read_to_string(Path::new(worktree).join(".gitignore")).ok()
}

#[test]
fn begin_creates_a_gitignore_with_the_jidoka_and_scratch_folders() {
    let project = Project::new();
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("Ignore"), &api, None, None).unwrap();
    assert_eq!(
        gitignore_of(&begun.worktree).as_deref(),
        Some(".jidoka/\n.scratch/\n")
    );
    // Only the new file differs from the base.
    assert_eq!(
        git(Path::new(&begun.worktree), &["status", "--porcelain"]),
        "?? .gitignore"
    );
}

// Creating symlinks on Windows needs a privilege that CI and users often lack, so this runs on Unix only.
#[test]
fn a_blocked_begin_leaves_no_gitignore_and_the_retry_resumes() {
    let project = Project::new();
    let mut failing = options("Retry me");
    failing.branch = Some("retry-me".into());
    failing.base = Some("nope".into());
    blocked(begin(
        &project.repo,
        &failing,
        &FakeApi::default(),
        None,
        None,
    ));
    assert!(!project.worktrees.join("retry-me").exists());
    assert!(!project.repo.join(".gitignore").exists());
    let api = api_with_run("Retry me");
    let begun = begin(
        &project.repo,
        &resuming("Retry me", "retry-me"),
        &api,
        None,
        None,
    )
    .unwrap();
    assert!(gitignore_of(&begun.worktree).is_some());
}

#[test]
fn an_explicit_base_and_branch_are_used_as_given() {
    let project = Project::new();
    let api = FakeApi::default();
    let mut opts = options("Anything");
    opts.base = Some("release".into());
    opts.branch = Some("feat/x".into());
    let begun = begin(&project.repo, &opts, &api, None, None).unwrap();
    assert_eq!(
        PathBuf::from(&begun.worktree),
        project.worktrees.join("feat-x")
    );
    assert_eq!(
        (begun.branch.as_str(), begun.base.as_str()),
        ("feat/x", "release")
    );
    assert_eq!(begun.base_commit, project.origin_commit("release"));
    assert_eq!(
        git(Path::new(&begun.worktree), &["branch", "--show-current"]),
        "feat/x"
    );
}

fn blocked(result: Result<Begun, BeginError>) -> (String, String, String) {
    match result {
        Err(BeginError::Blocked {
            run, reason, retry, ..
        }) => (run, reason, retry),
        other => panic!("expected a blocked run, got {other:?}"),
    }
}

fn assert_marked_blocked(api: &FakeApi, reason: &str) {
    let updates = api.updates.borrow();
    let (id, status, args) = updates.last().expect("the run was updated");
    assert_eq!((id.as_str(), status.as_str()), ("100", "blocked"));
    assert_eq!(
        value_after(args, "--blocking-reason").as_deref(),
        Some(reason)
    );
}

#[test]
fn outside_a_repository_no_run_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let api = FakeApi::default();
    let result = begin(dir.path(), &options("Nowhere"), &api, None, None);
    assert!(
        matches!(result, Err(BeginError::NoRun(ref text)) if text.contains("repository")),
        "{result:?}"
    );
    assert!(api.created.borrow().is_empty());
}

// --- collisions and retries ---------------------------------------------------------------------------

fn resuming(title: &str, branch: &str) -> Options {
    Options {
        title: String::new(),
        run: Some("100".into()),
        branch: Some(branch.into()),
        ..options(title)
    }
}

fn api_with_run(title: &str) -> FakeApi {
    let api = FakeApi::default();
    *api.existing.borrow_mut() = Some(RunSummary {
        title: title.into(),
        finished: false,
        worktree: None,
    });
    api
}

#[test]
fn an_explicit_branch_that_exists_stops_the_command_and_changes_nothing() {
    let project = Project::new();
    let before = git(&project.repo, &["rev-parse", "develop"]);
    let api = FakeApi::default();
    let mut opts = options("Explicit");
    opts.branch = Some("develop".into());
    let (_, reason, _) = blocked(begin(&project.repo, &opts, &api, None, None));
    assert!(
        reason.contains("develop") && reason.contains("already exists"),
        "{reason}"
    );
    assert_eq!(git(&project.repo, &["rev-parse", "develop"]), before);
    assert!(!project.worktrees.exists());
    assert_marked_blocked(&api, &reason);
}

#[test]
fn a_retry_after_a_run_without_a_worktree_completes_the_workspace() {
    let project = Project::new();
    let first = FakeApi::default();
    let mut failing = options("Retry me");
    failing.branch = Some("retry-me".into());
    failing.base = Some("nope".into());
    let (run, _, retry) = blocked(begin(&project.repo, &failing, &first, None, None));
    assert_eq!(run, "100");
    assert!(retry.contains("--branch retry-me"), "{retry}");

    let api = api_with_run("Retry me");
    let begun = begin(
        &project.repo,
        &resuming("Retry me", "retry-me"),
        &api,
        None,
        None,
    )
    .unwrap();
    assert_eq!(begun.run, "100");
    assert!(PathBuf::from(&begun.worktree).join(".git").exists());
    assert!(api.created.borrow().is_empty(), "no second run");
    let updates = api.updates.borrow();
    assert_eq!(updates.last().unwrap().1, "running");
}

#[test]
fn a_retry_that_finds_anything_else_stops_lists_it_and_changes_nothing() {
    // A plain folder where the worktree should be.
    let project = Project::new();
    let path = project.worktrees.join("odd");
    fs::create_dir_all(&path).unwrap();
    let api = api_with_run("Odd");
    let (_, reason, _) = blocked(begin(
        &project.repo,
        &resuming("Odd", "odd"),
        &api,
        None,
        None,
    ));
    assert!(
        reason.contains("not a worktree") && reason.contains(path.to_str().unwrap()),
        "{reason}"
    );
    assert!(path.exists());

    // A branch with commits of its own.
    let project = Project::new();
    git(&project.repo, &["branch", "bz", "origin/develop"]);
    git(&project.repo, &["checkout", "-q", "bz"]);
    git(
        &project.repo,
        &["commit", "-q", "--allow-empty", "-m", "own work"],
    );
    git(&project.repo, &["checkout", "-q", "develop"]);
    let tip = git(&project.repo, &["rev-parse", "bz"]);
    let api = api_with_run("Commits");
    let (_, reason, _) = blocked(begin(
        &project.repo,
        &resuming("Commits", "bz"),
        &api,
        None,
        None,
    ));
    assert!(
        reason.contains("bz") && reason.contains("commits"),
        "{reason}"
    );
    assert_eq!(git(&project.repo, &["rev-parse", "bz"]), tip);
    assert!(!project.worktrees.exists());

    // A worktree with uncommitted changes.
    let project = Project::new();
    let path = project.worktrees.join("dirty");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "dirty",
            path.to_str().unwrap(),
            "origin/develop",
        ],
    );
    fs::write(path.join("work.txt"), "unsaved").unwrap();
    let api = api_with_run("Dirty");
    let (_, reason, _) = blocked(begin(
        &project.repo,
        &resuming("Dirty", "dirty"),
        &api,
        None,
        None,
    ));
    assert!(reason.contains("uncommitted"), "{reason}");
    assert_eq!(
        fs::read_to_string(path.join("work.txt")).unwrap(),
        "unsaved"
    );
}

#[test]
fn resuming_an_unknown_or_finished_run_or_one_with_a_workspace_is_refused() {
    let project = Project::new();
    let unknown = FakeApi::default();
    let result = begin(&project.repo, &resuming("T", "t"), &unknown, None, None);
    assert!(matches!(result, Err(BeginError::NoRun(_))), "{result:?}");

    let finished = api_with_run("T");
    finished.existing.borrow_mut().as_mut().unwrap().finished = true;
    let result = begin(&project.repo, &resuming("T", "t"), &finished, None, None);
    assert!(
        matches!(result, Err(BeginError::NoRun(ref t)) if t.contains("finished")),
        "{result:?}"
    );

    let has = api_with_run("T");
    has.existing.borrow_mut().as_mut().unwrap().worktree = Some("/w/x".into());
    let result = begin(&project.repo, &resuming("T", "t"), &has, None, None);
    assert!(
        matches!(result, Err(BeginError::NoRun(ref t)) if t.contains("/w/x")),
        "{result:?}"
    );
    assert!(!project.worktrees.exists());
}

#[test]
fn nothing_in_the_command_deletes_resets_checks_out_or_cleans() {
    let project = Project::new();
    // The checkout holds work that a reset, clean, checkout or stash would destroy or move away.
    fs::write(project.repo.join("a.txt"), "committed\n").unwrap();
    git(&project.repo, &["add", "a.txt"]);
    git(&project.repo, &["commit", "-q", "-m", "local work"]);
    git(&project.repo, &["branch", "keep"]);
    git(
        &project.repo,
        &["worktree", "add", "-q", "../other", "keep"],
    );
    fs::write(project.repo.join("a.txt"), "edited\n").unwrap();
    fs::write(project.repo.join("untracked.txt"), "mine\n").unwrap();
    // A branch that exists without a worktree: a resume gives it one.
    git(&project.repo, &["branch", "bx", "origin/develop"]);

    // A fresh run, a second run whose name collides with the first, and a resume of a run without a worktree.
    let mut previous = Snapshot::of(&project);
    for (step, run_options) in [
        ("a fresh run", options("Add login")),
        ("a name collision", options("Add login")),
        ("a resume", resuming("Add login", "bx")),
    ] {
        let step_api = if step == "a resume" {
            api_with_run("Add login")
        } else {
            FakeApi::default()
        };
        begin(&project.repo, &run_options, &step_api, None, None)
            .unwrap_or_else(|error| panic!("{step}: {error:?}"));
        let now = Snapshot::of(&project);
        previous.assert_only_added_branches_and_worktrees(&now, step);
        previous = now;
    }
}

/// What a run start must leave as it was, plus the branches and worktrees it may add.
struct Snapshot {
    head: String,
    tips: String,
    status: String,
    stashes: String,
    branches: String,
    worktrees: String,
    tracked_file: String,
    untracked_file: String,
    origin_develop: String,
    origin_release: String,
}

impl Snapshot {
    fn of(project: &Project) -> Self {
        Self {
            head: git(&project.repo, &["symbolic-ref", "HEAD"]),
            tips: git(&project.repo, &["rev-parse", "HEAD", "keep"]),
            status: git(&project.repo, &["status", "--porcelain"]),
            stashes: git(&project.repo, &["stash", "list"]),
            branches: git(&project.repo, &["branch", "--list"]),
            worktrees: git(&project.repo, &["worktree", "list", "--porcelain"]),
            tracked_file: fs::read_to_string(project.repo.join("a.txt")).unwrap(),
            untracked_file: fs::read_to_string(project.repo.join("untracked.txt")).unwrap(),
            origin_develop: project.origin_commit("develop"),
            origin_release: project.origin_commit("release"),
        }
    }

    /// Everything is the same in `after`, except that branches and worktrees may have been added.
    fn assert_only_added_branches_and_worktrees(&self, after: &Self, step: &str) {
        assert_eq!(self.head, after.head, "{step}: HEAD");
        assert_eq!(self.tips, after.tips, "{step}: branch tips");
        assert_eq!(self.status, after.status, "{step}: status");
        assert_eq!(self.stashes, after.stashes, "{step}: stashes");
        assert_eq!(
            self.tracked_file, after.tracked_file,
            "{step}: tracked file"
        );
        assert_eq!(
            self.untracked_file, after.untracked_file,
            "{step}: untracked file"
        );
        assert_eq!(self.origin_develop, after.origin_develop, "{step}: develop");
        assert_eq!(self.origin_release, after.origin_release, "{step}: release");
        for line in self.branches.lines() {
            assert!(
                after.branches.contains(line.trim()),
                "{step}: branch {line} is gone"
            );
        }
        assert!(
            after.worktrees.starts_with(self.worktrees.as_str()),
            "{step}: worktrees\n{}\n---\n{}",
            self.worktrees,
            after.worktrees
        );
    }
}

// --- rework 1 ---------------------------------------------------------------------------------------

// --- rework 2 ---------------------------------------------------------------------------------------

/// The options a printed retry command stands for.
fn parsed_retry(retry: &str) -> Options {
    let words: Vec<&str> = retry.split_whitespace().collect();
    assert_eq!(&words[..4], ["jidoka", "run", "begin", "--run"], "{retry}");
    parse_resume(words[4], &words[5..]).unwrap()
}

#[test]
fn the_retry_after_a_taken_name_never_takes_over_the_workspace_that_holds_it() {
    let project = Project::new();
    let api = FakeApi::default();
    // Run A holds a clean, unused worktree named feat; a plain branch is taken as well.
    let mut mine = options("A");
    mine.branch = Some("feat".into());
    let a = begin(&project.repo, &mine, &api, None, None).unwrap();
    git(&project.repo, &["branch", "feat2", "origin/develop"]);

    for taken in ["feat", "feat2"] {
        let api_b = FakeApi::default();
        let mut wants = options("B work");
        wants.branch = Some(taken.into());
        let (_, reason, retry) = blocked(begin(&project.repo, &wants, &api_b, None, None));
        assert!(reason.contains("already exists"), "{reason}");
        assert!(!retry.contains(&format!("--branch {taken}")), "{retry}");

        // Running the printed command makes a workspace of its own.
        let resume_api = api_with_run("B work");
        let begun = begin(
            &project.repo,
            &parsed_retry(&retry),
            &resume_api,
            None,
            None,
        )
        .unwrap();
        assert_ne!(begun.worktree, a.worktree);
        assert!(begun.branch.starts_with("b-work"), "{}", begun.branch);
    }
    // Run A's workspace is untouched.
    assert_eq!(
        git(Path::new(&a.worktree), &["branch", "--show-current"]),
        "feat"
    );
}

// --- rework 3 ---------------------------------------------------------------------------------------

fn api_with_session_run(id: &str, title: &str) -> FakeApi {
    let api = FakeApi::default();
    api.session_runs
        .borrow_mut()
        .push((id.into(), title.into()));
    api
}

fn session(id: &str, agent: Option<&'static str>) -> AgentSession {
    AgentSession {
        id: id.into(),
        agent,
    }
}

#[test]
fn a_session_already_on_another_run_gives_a_warning_naming_the_agents_command() {
    let cases = [
        (Some("claude"), "use /clear in Claude Code"),
        (Some("codex"), "use /new in Codex"),
        (Some("copilot"), "use /clear in Copilot CLI"),
        (None, "Start each run in a fresh agent session."),
    ];
    for (agent, expected) in cases {
        let project = Project::new();
        let api = api_with_session_run("7", "Earlier work");
        let current = session("s", agent);
        let begun = begin(&project.repo, &options("New"), &api, Some(current), None).unwrap();
        let warning = begun.session_warning.clone().unwrap();
        assert!(warning.contains("7 (Earlier work)"), "{warning}");
        assert!(warning.contains(expected), "{warning}");
        if agent.is_none() {
            assert!(!warning.contains('/'), "{warning}");
        }
        assert!(render(&begun, "/p/plans/100.md").contains("Warning: This agent session"));
        let json = serde_json::to_value(&begun).unwrap();
        assert_eq!(json["session_warning"], warning);
    }
}
