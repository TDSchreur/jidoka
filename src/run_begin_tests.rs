//! `factory run begin` with a fake server and real git in temporary repositories with a local bare remote.

use super::*;
use std::{cell::RefCell, fs, path::PathBuf, process::Command};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
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

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
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
        let repo = root.join("repo");
        let clone = Command::new("git")
            .args(["clone", "-q"])
            .arg(&remote)
            .arg(&repo)
            .output()
            .unwrap();
        assert!(
            clone.status.success(),
            "{}",
            String::from_utf8_lossy(&clone.stderr)
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
        Err(BeginError::Blocked { run, reason, retry }) => (run, reason, retry),
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
    };
    let text = render(&begun);
    assert!(text.starts_with("Run: 100\n"), "{text}");
    for part in ["/w/repo.worktrees/x", "x", "develop", "abc123"] {
        assert!(text.contains(part), "{part}: {text}");
    }
    let json = serde_json::to_value(&begun).unwrap();
    assert_eq!(json["run"], "100");
    assert_eq!(json["base_commit"], "abc123");
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
        "\"remove\"",
        "remove_dir",
        "remove_file",
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
