//! `factory run begin` with a fake server and real git in temporary repositories with a local bare remote.

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
    std::env::temp_dir().join("factory-tests-no-such-gitconfig")
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
type Update = (String, String, String, Vec<String>);

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

    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String> {
        if self.fail_updates.get() {
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
    opts.mode = Some("factory-full".into());
    let begun = begin(&project.repo, &opts, &api, Some("sess-1".into()), None).unwrap();

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
    let (id, stage, status, args) = updates.last().unwrap();
    assert_eq!(
        (id.as_str(), stage.as_str(), status.as_str()),
        ("100", "define", "running")
    );
    assert_eq!(value_after(args, "--mode").as_deref(), Some("factory-full"));
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
fn begin_creates_a_gitignore_with_the_factory_and_scratch_folders() {
    let project = Project::new();
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("Ignore"), &api, None, None).unwrap();
    assert_eq!(
        gitignore_of(&begun.worktree).as_deref(),
        Some(".factory/\n.scratch/\n")
    );
    // Only the new file differs from the base.
    assert_eq!(
        git(Path::new(&begun.worktree), &["status", "--porcelain"]),
        "?? .gitignore"
    );
}

#[test]
fn begin_reports_the_added_ignore_entries() {
    let project = Project::new();
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("Ignore"), &api, None, None).unwrap();
    assert_eq!(begun.gitignore_added, [".factory/", ".scratch/"]);
    assert_eq!(begun.gitignore_warning, None);
    assert!(render(&begun, "/p/plans/100.md").contains("Added to .gitignore"));
}

#[test]
fn an_unreadable_gitignore_is_a_warning_not_a_failed_begin() {
    let project = Project::new();
    fs::create_dir(project.repo.join(".gitignore")).unwrap();
    fs::write(project.repo.join(".gitignore/keep"), "x").unwrap();
    git(&project.repo, &["add", ".gitignore/keep"]);
    git(&project.repo, &["commit", "-q", "-m", "dir"]);
    git(&project.repo, &["push", "-q", "origin", "develop"]);
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("Ignore"), &api, None, None).unwrap();
    assert!(begun.gitignore_added.is_empty());
    assert!(begun.gitignore_warning.is_some());
    // Reading a directory fails with EISDIR; the "os error" suffix is not localised.
    // The EISDIR code (21) is the Unix errno; Windows reports a different code, so only Unix checks it.
    #[cfg(unix)]
    assert!(
        begun
            .gitignore_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("(os error 21)")),
        "{:?}",
        begun.gitignore_warning
    );
}

// Creating symlinks on Windows needs a privilege that CI and users often lack, so this runs on Unix only.
#[cfg(unix)]
#[test]
fn a_symlinked_gitignore_is_a_warning_and_its_target_is_untouched() {
    let project = Project::new();
    fs::write(project.repo.join("elsewhere"), "secret\n").unwrap();
    std::os::unix::fs::symlink("elsewhere", project.repo.join(".gitignore")).unwrap();
    git(&project.repo, &["add", "elsewhere", ".gitignore"]);
    git(&project.repo, &["commit", "-q", "-m", "link"]);
    git(&project.repo, &["push", "-q", "origin", "develop"]);
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("Ignore"), &api, None, None).unwrap();
    assert!(begun.gitignore_added.is_empty());
    assert!(begun.gitignore_warning.unwrap().contains("symlink"));
    let target = Path::new(&begun.worktree).join("elsewhere");
    assert_eq!(fs::read_to_string(target).unwrap(), "secret\n");
}

#[test]
fn begin_appends_to_a_tracked_gitignore_and_shows_only_that_change() {
    let project = Project::new();
    fs::write(project.repo.join(".gitignore"), "/target/").unwrap();
    git(&project.repo, &["add", ".gitignore"]);
    git(&project.repo, &["commit", "-q", "-m", "ignore"]);
    git(&project.repo, &["push", "-q", "origin", "develop"]);
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("Ignore"), &api, None, None).unwrap();
    assert_eq!(
        gitignore_of(&begun.worktree).as_deref(),
        Some("/target/\n.factory/\n.scratch/\n")
    );
    assert_eq!(
        git(Path::new(&begun.worktree), &["status", "--porcelain"]),
        "M .gitignore"
    );
}

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

#[test]
fn the_base_is_fetched_first_so_the_worktree_starts_from_the_current_remote() {
    let project = Project::new();
    // The remote moves on after the clone.
    let other = project.worktrees.parent().unwrap().join("other");
    git(
        project.worktrees.parent().unwrap(),
        &["clone", "-q", project.remote.to_str().unwrap(), "other"],
    );
    git(&other, &["commit", "-q", "--allow-empty", "-m", "third"]);
    git(&other, &["push", "-q", "origin", "develop"]);
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("Fresh base"), &api, None, None).unwrap();
    assert_eq!(begun.base_commit, project.origin_commit("develop"));
}

#[test]
fn the_worktree_goes_beside_the_main_checkout_whichever_folder_the_command_runs_in() {
    let project = Project::new();
    let api = FakeApi::default();
    let first = begin(&project.repo, &options("First"), &api, None, None).unwrap();
    let linked = PathBuf::from(&first.worktree);
    let sub = linked.join("nested");
    fs::create_dir(&sub).unwrap();
    let second = begin(&sub, &options("Second"), &api, None, None).unwrap();
    assert_eq!(
        PathBuf::from(&second.worktree),
        project.worktrees.join("second")
    );
}

#[test]
fn names_come_from_the_title() {
    let project = Project::new();
    let api = FakeApi::default();
    for (title, name) in [
        ("Add `factory run begin`!", "add-factory-run-begin"),
        ("  Fix   the   BUG  ", "fix-the-bug"),
        (
            "Update the README, then rewrite the whole documentation tree thoroughly",
            "update-the-readme-then-rewrite-the-whole",
        ),
        ("Café ünïcode", "caf-n-code"),
    ] {
        let begun = begin(&project.repo, &options(title), &api, None, None).unwrap();
        assert_eq!(begun.branch, name, "{title}");
        assert!(begun.branch.len() <= 40);
    }
    // A title without a letter or digit still gets a name.
    let begun = begin(&project.repo, &options("¿¡?!"), &api, None, None).unwrap();
    assert_eq!(begun.branch, "run-100");
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
    let (id, stage, status, args) = updates.last().expect("the run was updated");
    assert_eq!(
        (id.as_str(), stage.as_str(), status.as_str()),
        ("100", "define", "blocked")
    );
    assert_eq!(
        value_after(args, "--blocking-reason").as_deref(),
        Some(reason)
    );
}

#[test]
fn a_missing_origin_blocks_the_run_with_the_reason_and_the_retry() {
    let project = Project::new();
    git(&project.repo, &["remote", "remove", "origin"]);
    let api = FakeApi::default();
    let (run, reason, retry) = blocked(begin(
        &project.repo,
        &options("No remote"),
        &api,
        None,
        None,
    ));
    assert_eq!(run, "100");
    assert!(reason.contains("origin"), "{reason}");
    assert!(retry.starts_with("factory run begin --run 100"), "{retry}");
    assert_marked_blocked(&api, &reason);
    assert!(!project.worktrees.exists());
}

#[test]
fn an_unknown_base_blocks_the_run() {
    let project = Project::new();
    let api = FakeApi::default();
    let mut opts = options("Unknown base");
    opts.base = Some("nope".into());
    let (_, reason, retry) = blocked(begin(&project.repo, &opts, &api, None, None));
    assert!(
        reason.contains("nope") && reason.contains("origin"),
        "{reason}"
    );
    assert!(retry.contains("--base nope"), "{retry}");
    assert_marked_blocked(&api, &reason);
    assert!(!project.worktrees.exists());
}

#[test]
fn without_origin_head_and_without_a_base_the_reason_says_to_pass_one() {
    let project = Project::new();
    git(&project.repo, &["remote", "set-head", "origin", "-d"]);
    let api = FakeApi::default();
    let (_, reason, _) = blocked(begin(&project.repo, &options("No head"), &api, None, None));
    assert!(reason.contains("--base"), "{reason}");
    assert_marked_blocked(&api, &reason);
}

#[test]
fn a_failed_fetch_blocks_the_run() {
    let project = Project::new();
    git(
        &project.repo,
        &[
            "remote",
            "set-url",
            "origin",
            project.remote.join("missing").to_str().unwrap(),
        ],
    );
    let api = FakeApi::default();
    let mut opts = options("Offline");
    opts.base = Some("develop".into());
    let (_, reason, _) = blocked(begin(&project.repo, &opts, &api, None, None));
    assert!(reason.to_lowercase().contains("fetch"), "{reason}");
    assert_marked_blocked(&api, &reason);
}

#[cfg(unix)]
#[test]
fn git_that_cannot_write_blocks_the_run_and_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    fs::create_dir(&project.worktrees).unwrap();
    fs::set_permissions(&project.worktrees, fs::Permissions::from_mode(0o555)).unwrap();
    // A user who may write anyway (root) cannot show the failure.
    let probe = project.worktrees.join("probe");
    if fs::create_dir(&probe).is_ok() {
        fs::remove_dir(&probe).unwrap();
        fs::set_permissions(&project.worktrees, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: this user can write to a read-only folder (root)");
        return;
    }
    let api = FakeApi::default();
    let result = begin(&project.repo, &options("Read only"), &api, None, None);
    fs::set_permissions(&project.worktrees, fs::Permissions::from_mode(0o755)).unwrap();
    let (_, reason, _) = blocked(result);
    assert!(reason.to_lowercase().contains("permission"), "{reason}");
    assert_marked_blocked(&api, &reason);
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

#[test]
fn the_output_has_the_run_first_and_the_same_fields_in_json() {
    let begun = Begun {
        run: "100".into(),
        worktree: "/w/repo.worktrees/x".into(),
        branch: "x".into(),
        base: "develop".into(),
        base_commit: "abc123".into(),
        gitignore_added: vec![".factory/".into()],
        gitignore_warning: None,
        session_warning: None,
    };
    let text = render(&begun, "/p/plans/100.md");
    assert!(text.starts_with("Run: 100\n"), "{text}");
    for part in ["/w/repo.worktrees/x", "x", "develop", "abc123"] {
        assert!(text.contains(part), "{part}: {text}");
    }
    let json = serde_json::to_value(&begun).unwrap();
    assert_eq!(json["run"], "100");
    assert_eq!(json["base_commit"], "abc123");
    assert_eq!(json["gitignore_added"][0], ".factory/");
}

#[test]
fn begin_shows_the_plan_path_in_text_and_json() {
    let begun = Begun {
        run: "100".into(),
        worktree: "/w/x".into(),
        branch: "x".into(),
        base: "develop".into(),
        base_commit: "abc123".into(),
        gitignore_added: Vec::new(),
        gitignore_warning: None,
        session_warning: None,
    };
    let text = render(&begun, "/data/plans/100.md");
    assert!(text.contains("Plan: /data/plans/100.md\n"), "{text}");
    let json = serde_json::to_value(Output {
        begun: &begun,
        plan: "/data/plans/100.md".into(),
    })
    .unwrap();
    assert_eq!(json["plan"], "/data/plans/100.md");
    assert_eq!(json["run"], "100");
}

#[test]
fn the_options_are_checked_before_anything_is_created() {
    let ok = parse_options(
        "T",
        &[
            "--mode",
            "factory-fast",
            "--base",
            "main",
            "--branch",
            "b",
            "--json",
        ],
    )
    .unwrap();
    assert_eq!(ok.mode.as_deref(), Some("factory-fast"));
    assert_eq!(
        (ok.base.as_deref(), ok.branch.as_deref(), ok.json),
        (Some("main"), Some("b"), true)
    );
    assert_eq!(parse_options("T", &[]).unwrap().title, "T");
    for bad in [
        vec!["--mode", "turbo"],
        vec!["--mode"],
        vec!["--base"],
        vec!["--base", "-x"],
        vec!["--branch", "-x"],
        vec!["--branch", "a b"],
        vec!["--branch", "a..b"],
        vec!["--branch", "x.lock"],
        vec!["--json", "--json"],
        vec!["--bogus"],
        vec!["--base", "a", "--base", "b"],
    ] {
        assert!(parse_options("T", &bad).is_err(), "{bad:?}");
    }
    assert!(parse_options("", &[]).is_err());
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
fn two_runs_with_the_same_title_get_different_branches_and_paths() {
    let project = Project::new();
    let api = FakeApi::default();
    let first = begin(&project.repo, &options("Same title"), &api, None, None).unwrap();
    let second = begin(&project.repo, &options("Same title"), &api, None, None).unwrap();
    let third = begin(&project.repo, &options("Same title"), &api, None, None).unwrap();
    assert_eq!(
        [
            first.branch.as_str(),
            second.branch.as_str(),
            third.branch.as_str()
        ],
        ["same-title", "same-title-2", "same-title-3"]
    );
    assert!(PathBuf::from(&third.worktree).ends_with("repo.worktrees/same-title-3"));
    // A name that is taken by a plain folder is skipped too.
    fs::create_dir_all(project.worktrees.join("other-title")).unwrap();
    let other = begin(&project.repo, &options("Other title"), &api, None, None).unwrap();
    assert_eq!(other.branch, "other-title-2");
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
fn an_explicit_path_in_the_way_stops_the_command_and_leaves_it_alone() {
    let project = Project::new();
    let taken = project.worktrees.join("mine");
    fs::create_dir_all(&taken).unwrap();
    fs::write(taken.join("keep.txt"), "x").unwrap();
    let api = FakeApi::default();
    let mut opts = options("Explicit");
    opts.branch = Some("mine".into());
    let (_, reason, _) = blocked(begin(&project.repo, &opts, &api, None, None));
    assert!(reason.contains(taken.to_str().unwrap()), "{reason}");
    assert!(taken.join("keep.txt").exists());
    assert_eq!(git(&project.repo, &["branch", "--list", "mine"]), "");
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
    assert_eq!(updates.last().unwrap().2, "running");
}

#[test]
fn a_retry_adds_the_worktree_of_a_branch_that_has_none() {
    let project = Project::new();
    git(&project.repo, &["branch", "bx", "origin/develop"]);
    let tip = git(&project.repo, &["rev-parse", "bx"]);
    let api = api_with_run("Branch only");
    let begun = begin(
        &project.repo,
        &resuming("Branch only", "bx"),
        &api,
        None,
        None,
    )
    .unwrap();
    assert_eq!(begun.branch, "bx");
    assert_eq!(begun.base_commit, tip);
    assert_eq!(
        git(Path::new(&begun.worktree), &["branch", "--show-current"]),
        "bx"
    );
}

#[test]
fn a_retry_only_records_a_clean_worktree_that_was_not_recorded() {
    let project = Project::new();
    let path = project.worktrees.join("by");
    git(
        &project.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "by",
            path.to_str().unwrap(),
            "origin/develop",
        ],
    );
    let api = api_with_run("Not recorded");
    let begun = begin(
        &project.repo,
        &resuming("Not recorded", "by"),
        &api,
        None,
        None,
    )
    .unwrap();
    assert_eq!(PathBuf::from(&begun.worktree), path);
    let updates = api.updates.borrow();
    let args = &updates.last().unwrap().3;
    assert_eq!(value_after(args, "--worktree").as_deref(), path.to_str());
    // No second worktree was made.
    assert_eq!(fs::read_dir(&project.worktrees).unwrap().count(), 1);
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
    let source = include_str!("run_begin.rs");
    for forbidden in [
        "\"reset\"",
        "\"clean\"",
        "\"checkout\"",
        "\"restore\"",
        "\"rm\"",
        "\"prune\"",
        "\"stash\"",
        "\"--force\"",
        "\"-D\"",
        "\"-d\"",
        "\"-B\"",
        "\"-f\"",
        "\"-ff\"",
        "\"--hard\"",
        "\"--detach\"",
        "\"remove\"",
        "\"switch\"",
        "\"gc\"",
        "\"update-ref\"",
        "remove_dir",
        "remove_file",
        "fs::write",
    ] {
        assert!(!source.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn a_resume_is_parsed_without_a_title() {
    let parsed = parse_resume("100", &["--base", "main", "--branch", "b"]).unwrap();
    assert_eq!(
        (
            parsed.run.as_deref(),
            parsed.base.as_deref(),
            parsed.branch.as_deref()
        ),
        (Some("100"), Some("main"), Some("b"))
    );
    assert!(parse_resume("12x", &[]).is_err());
    assert!(parse_resume("", &[]).is_err());
}

// --- rework 1 ---------------------------------------------------------------------------------------

#[test]
fn a_resume_without_a_branch_never_takes_over_an_existing_workspace() {
    let project = Project::new();
    let api = FakeApi::default();
    let first = begin(&project.repo, &options("Add login"), &api, None, None).unwrap();
    assert_eq!(first.branch, "add-login");

    // Another run with the same title is resumed without a branch: nothing was made for it yet.
    let other = api_with_run("Add login");
    let opts = Options {
        title: String::new(),
        run: Some("200".into()),
        ..Options::default()
    };
    let resumed = begin(&project.repo, &opts, &other, None, None).unwrap();
    assert_eq!(resumed.branch, "add-login-2");
    assert_ne!(resumed.worktree, first.worktree);
    // The first run's workspace is untouched.
    assert_eq!(
        git(Path::new(&first.worktree), &["branch", "--show-current"]),
        "add-login"
    );
}

#[test]
fn another_base_can_be_fetched_in_a_single_branch_clone() {
    let project = Project::new();
    let single = project.worktrees.parent().unwrap().join("single");
    let mut clone = Command::new("git");
    clone
        .args(["clone", "-q", "--single-branch", "-b", "develop"])
        .arg(&project.remote)
        .arg(&single);
    assert!(isolated(&mut clone).output().unwrap().status.success());
    let api = FakeApi::default();
    let mut opts = options("Other base");
    opts.base = Some("release".into());
    let begun = begin(&single, &opts, &api, None, None).unwrap();
    assert_eq!(begun.base_commit, project.origin_commit("release"));
}

#[test]
fn a_worktree_git_lists_but_that_is_gone_says_to_prune_it() {
    let project = Project::new();
    let api = FakeApi::default();
    let mut opts = options("Gone");
    opts.branch = Some("gone".into());
    let made = begin(&project.repo, &opts, &api, None, None).unwrap();
    fs::remove_dir_all(&made.worktree).unwrap();
    let again = api_with_run("Gone");
    let (_, reason, _) = blocked(begin(
        &project.repo,
        &resuming("Gone", "gone"),
        &again,
        None,
        None,
    ));
    assert!(reason.contains("git worktree prune"), "{reason}");
    assert!(!reason.contains("not on gone"), "{reason}");
}

#[test]
fn a_repository_inside_another_git_folder_is_refused_before_a_run_is_made() {
    let dir = tempfile::tempdir().unwrap();
    let inside = dir.path().join("super/.git/modules/lib");
    fs::create_dir_all(&inside).unwrap();
    git(&inside, &["init", "-q", "--bare"]);
    let api = FakeApi::default();
    let result = begin(&inside, &options("Module"), &api, None, None);
    assert!(
        matches!(result, Err(BeginError::NoRun(ref t)) if t.contains(".git")),
        "{result:?}"
    );
    assert!(api.created.borrow().is_empty());
    assert!(!dir.path().join("super/.git/modules/lib.worktrees").exists());
}

#[test]
fn a_tag_with_the_name_of_the_branch_does_not_decide_whether_the_branch_is_unused() {
    let project = Project::new();
    git(&project.repo, &["branch", "bt", "origin/develop"]);
    // A tag of the same name on a commit that is not on the base.
    git(
        &project.repo,
        &["checkout", "-q", "--detach", "origin/develop"],
    );
    git(
        &project.repo,
        &["commit", "-q", "--allow-empty", "-m", "elsewhere"],
    );
    git(&project.repo, &["tag", "bt"]);
    git(&project.repo, &["checkout", "-q", "develop"]);
    let api = api_with_run("Tagged");
    let begun = begin(&project.repo, &resuming("Tagged", "bt"), &api, None, None).unwrap();
    assert_eq!(begun.branch, "bt");
}

#[test]
fn the_text_output_is_printable_whatever_the_folder_is_called() {
    let begun = Begun {
        run: "100".into(),
        worktree: "/w/\u{1b}[2J/x".into(),
        branch: "b".into(),
        base: "develop".into(),
        base_commit: "abc".into(),
        gitignore_added: Vec::new(),
        gitignore_warning: Some("bad \u{1b}[2J".into()),
        session_warning: None,
    };
    assert!(!render(&begun, "/p/plans/100.md")
        .chars()
        .any(|c| c.is_control() && c != '\n'));
}

#[test]
fn a_block_that_could_not_be_recorded_says_so() {
    let project = Project::new();
    git(&project.repo, &["remote", "remove", "origin"]);
    let api = FakeApi::default();
    api.fail_updates.set(true);
    match begin(&project.repo, &options("Offline"), &api, None, None) {
        Err(BeginError::Blocked { marked, .. }) => assert!(!marked),
        other => panic!("{other:?}"),
    }
}

#[test]
fn git_is_asked_in_english_so_that_its_messages_can_be_told_apart() {
    assert!(include_str!("run_begin.rs").contains(r#".env("LC_ALL", "C")"#));
}

// --- rework 2 ---------------------------------------------------------------------------------------

/// The options a printed retry command stands for.
fn parsed_retry(retry: &str) -> Options {
    let words: Vec<&str> = retry.split_whitespace().collect();
    assert_eq!(&words[..4], ["factory", "run", "begin", "--run"], "{retry}");
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

#[test]
fn a_retry_before_any_name_was_chosen_keeps_the_branch_that_was_asked_for() {
    let project = Project::new();
    let api = FakeApi::default();
    let mut wants = options("Early");
    wants.branch = Some("mine".into());
    wants.base = Some("nope".into());
    let (_, _, retry) = blocked(begin(&project.repo, &wants, &api, None, None));
    assert!(retry.contains("--branch mine"), "{retry}");
}

// --- rework 3 ---------------------------------------------------------------------------------------

#[test]
fn a_name_that_is_taken_is_found_before_anything_else_can_fail() {
    let project = Project::new();
    let api = FakeApi::default();
    let mut mine = options("A");
    mine.branch = Some("feat".into());
    let a = begin(&project.repo, &mine, &api, None, None).unwrap();

    // The fetch would fail (unknown base), but the name is taken from the start.
    let api_b = FakeApi::default();
    let mut wants = options("B work");
    wants.branch = Some("feat".into());
    wants.base = Some("nope".into());
    let (_, reason, retry) = blocked(begin(&project.repo, &wants, &api_b, None, None));
    assert!(reason.contains("already exists"), "{reason}");
    assert!(!retry.contains("--branch feat"), "{retry}");
    assert!(retry.contains("--base nope"), "{retry}");

    // Fixing the base and running the printed retry makes a workspace of its own.
    let mut fixed = parsed_retry(&retry);
    fixed.base = Some("develop".into());
    let resume_api = api_with_run("B work");
    let begun = begin(&project.repo, &fixed, &resume_api, None, None).unwrap();
    assert_ne!(begun.worktree, a.worktree);
}

fn api_with_session_run(id: &str, title: &str) -> FakeApi {
    let api = FakeApi::default();
    api.session_runs
        .borrow_mut()
        .push((id.into(), title.into()));
    api
}

#[test]
fn a_session_already_on_another_run_gives_a_warning() {
    let project = Project::new();
    let api = api_with_session_run("7", "Earlier work");
    let begun = begin(&project.repo, &options("New"), &api, Some("s".into()), None).unwrap();
    let warning = begun.session_warning.clone().unwrap();
    assert!(warning.contains("7 (Earlier work)"), "{warning}");
    assert!(warning.contains("/clear"), "{warning}");
    assert!(render(&begun, "/p/plans/100.md").contains("Warning: This agent session"));
    let json = serde_json::to_value(&begun).unwrap();
    assert_eq!(json["session_warning"], warning);
}

#[test]
fn no_session_id_or_no_other_run_gives_no_warning() {
    let project = Project::new();
    let api = api_with_session_run("7", "Earlier work");
    let begun = begin(&project.repo, &options("A"), &api, None, None).unwrap();
    assert_eq!(begun.session_warning, None);

    let project = Project::new();
    let api = FakeApi::default();
    let begun = begin(&project.repo, &options("B"), &api, Some("s".into()), None).unwrap();
    assert_eq!(begun.session_warning, None);
}

#[test]
fn a_resumed_run_is_not_counted_against_itself() {
    let project = Project::new();
    let api = api_with_session_run("55", "Resumed");
    *api.existing.borrow_mut() = Some(RunSummary {
        title: "Resumed".into(),
        finished: false,
        worktree: None,
    });
    let mut opts = options("");
    opts.run = Some("55".into());
    let begun = begin(&project.repo, &opts, &api, Some("s".into()), None).unwrap();
    assert_eq!(begun.session_warning, None);
}

#[test]
fn a_list_failure_gives_no_warning_and_begin_succeeds() {
    let project = Project::new();
    let api = FakeApi::default();
    api.fail_listing.set(true);
    let begun = begin(&project.repo, &options("C"), &api, Some("s".into()), None).unwrap();
    assert_eq!(begun.session_warning, None);
}
