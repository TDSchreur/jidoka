//! `factory run check` with real git in temporary repositories and a local bare remote.

use super::*;
use std::{fs, process::Command};
use tempfile::TempDir;

/// The plain path form git prints; on Windows `canonicalize` gives a `\\?\` path that git reads as a URL.
fn real_path(path: &Path) -> PathBuf {
    canonical_dir(path).unwrap()
}

/// Git without the user's own settings, with a fixed author so that commits can be made.
fn isolated(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("factory-tests-no-such-gitconfig"),
        )
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
}

fn output(dir: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false"])
        .args(args);
    isolated(&mut command).output().unwrap()
}

fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = output(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The code's git, isolated like the tests' own. It records every command so a test can see that the
/// check only reads.
#[derive(Default)]
struct TestGit {
    commands: std::cell::RefCell<Vec<Vec<String>>>,
}

impl Git for TestGit {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        self.commands
            .borrow_mut()
            .push(args.iter().map(|a| a.to_string()).collect());
        let out = output(dir, args);
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
        if out.status.success() {
            Ok(text(&out.stdout))
        } else if out.stderr.is_empty() {
            Err(text(&out.stdout))
        } else {
            Err(text(&out.stderr))
        }
    }

    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        self.commands
            .borrow_mut()
            .push(args.iter().map(|a| a.to_string()).collect());
        output(dir, args)
            .status
            .code()
            .ok_or_else(|| "signal".to_string())
    }
}

/// A bare remote with `develop`, a clone `repo`, and a worktree `wt` on `feature` with one pushed commit.
struct Fixture {
    _tmp: TempDir,
    root: PathBuf,
    repo: PathBuf,
    wt: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = real_path(tmp.path());
        let remote = root.join("remote.git");
        let repo = root.join("repo");
        let wt = root.join("wt");
        fs::create_dir(&remote).unwrap();
        run_git(&remote, &["init", "-q", "--bare", "-b", "develop"]);
        run_git(
            &root,
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        );
        run_git(&repo, &["checkout", "-q", "-b", "develop"]);
        fs::write(repo.join("a.txt"), "a\n").unwrap();
        run_git(&repo, &["add", "a.txt"]);
        run_git(&repo, &["commit", "-q", "-m", "base"]);
        run_git(&repo, &["push", "-q", "origin", "develop"]);
        let base = run_git(&repo, &["rev-parse", "HEAD"]);
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        );
        fs::write(wt.join("b.txt"), "b\n").unwrap();
        run_git(&wt, &["add", "b.txt"]);
        run_git(&wt, &["commit", "-q", "-m", "feature work"]);
        run_git(&wt, &["push", "-q", "origin", "feature"]);
        Fixture {
            _tmp: tmp,
            root,
            repo,
            wt,
            base,
        }
    }

    fn recorded(&self) -> Recorded {
        Recorded {
            id: "7".into(),
            repository: Some(self.repo.display().to_string()),
            worktree: Some(self.wt.display().to_string()),
            branch: Some("feature".into()),
            base_commit: Some(self.base.clone()),
        }
    }
}

fn state(report: &Report, name: CheckName) -> &Check {
    report.checks.iter().find(|c| c.check == name).unwrap()
}

fn states(report: &Report) -> Vec<State> {
    report.checks.iter().map(|c| c.state).collect()
}

#[test]
fn a_matching_run_passes_every_check() {
    let fx = Fixture::new();
    let report = check(&fx.recorded(), &TestGit::default());
    assert_eq!(states(&report), vec![State::Matches; 5], "{report:#?}");
    assert!(report.consistent);
    assert_eq!(report.exit_code, 0);
    assert!(report.changes.is_empty());
    assert!(report.unpushed.is_empty());
    let text = render(&report);
    assert!(text.contains("matches the repository"), "{text}");
}

#[test]
fn unpushed_commits_are_listed_without_being_a_difference() {
    let fx = Fixture::new();
    fs::write(fx.wt.join("c.txt"), "c\n").unwrap();
    run_git(&fx.wt, &["add", "c.txt"]);
    run_git(&fx.wt, &["commit", "-q", "-m", "not pushed yet"]);
    let report = check(&fx.recorded(), &TestGit::default());
    assert!(report.consistent, "{report:#?}");
    assert_eq!(report.unpushed.len(), 1);
    assert!(report.unpushed[0].ends_with("not pushed yet"));
    assert!(render(&report).contains("Commits not pushed"));
}

#[test]
fn uncommitted_changes_differ_and_are_listed() {
    let fx = Fixture::new();
    fs::write(fx.wt.join("b.txt"), "changed\n").unwrap();
    fs::write(fx.wt.join("new.txt"), "new\n").unwrap();
    let report = check(&fx.recorded(), &TestGit::default());
    let working = state(&report, CheckName::WorkingTree);
    assert_eq!(working.state, State::Differs);
    assert_eq!(working.found, "2 uncommitted paths");
    assert_eq!(report.changes, vec!["M b.txt", "?? new.txt"]);
    assert_eq!(report.exit_code, DIFFERS_EXIT);
    // The check never stages or discards anything.
    assert_eq!(
        fs::read_to_string(fx.wt.join("b.txt")).unwrap(),
        "changed\n"
    );
    assert_eq!(run_git(&fx.wt, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn another_branch_differs_with_the_branch_found() {
    let fx = Fixture::new();
    run_git(&fx.wt, &["checkout", "-q", "-b", "other"]);
    let report = check(&fx.recorded(), &TestGit::default());
    let branch = state(&report, CheckName::Branch);
    assert_eq!(branch.state, State::Differs);
    assert_eq!(branch.found, "on other");
    assert_eq!(branch.recorded.as_deref(), Some("feature"));
    assert!(!report.consistent);
}

#[test]
fn a_detached_head_differs() {
    let fx = Fixture::new();
    run_git(&fx.wt, &["checkout", "-q", "--detach"]);
    let report = check(&fx.recorded(), &TestGit::default());
    let branch = state(&report, CheckName::Branch);
    assert_eq!(branch.state, State::Differs);
    assert!(branch.found.contains("detached"), "{}", branch.found);
}

#[test]
fn a_base_commit_that_is_not_an_ancestor_differs() {
    let fx = Fixture::new();
    // A commit on develop after the branch started is not in the branch.
    fs::write(fx.repo.join("d.txt"), "d\n").unwrap();
    run_git(&fx.repo, &["add", "d.txt"]);
    run_git(&fx.repo, &["commit", "-q", "-m", "later on develop"]);
    let mut run = fx.recorded();
    run.base_commit = Some(run_git(&fx.repo, &["rev-parse", "HEAD"]));
    let report = check(&run, &TestGit::default());
    let base = state(&report, CheckName::BaseCommit);
    assert_eq!(base.state, State::Differs);
    assert_eq!(base.found, "not an ancestor of HEAD");
}

#[test]
fn a_base_commit_unknown_to_git_differs() {
    let fx = Fixture::new();
    let mut run = fx.recorded();
    run.base_commit = Some("0123456789abcdef0123456789abcdef01234567".into());
    let report = check(&run, &TestGit::default());
    let base = state(&report, CheckName::BaseCommit);
    assert_eq!(base.state, State::Differs);
    assert_eq!(
        base.found,
        "not found in this repository, or the abbreviation is ambiguous"
    );
    // Anything but a commit id, such as an option, a branch or a search, is refused before git sees it.
    for value in ["--all", "HEAD", "develop", ":/base", "abc"] {
        run.base_commit = Some(value.into());
        let report = check(&run, &TestGit::default());
        let base = state(&report, CheckName::BaseCommit);
        assert_eq!(base.state, State::Differs, "{value}");
        assert!(base.found.starts_with("not a commit id"), "{value}");
    }
    // An abbreviated id is fine.
    run.base_commit = Some(fx.base[..10].to_string());
    let report = check(&run, &TestGit::default());
    assert_eq!(state(&report, CheckName::BaseCommit).state, State::Matches);
}

#[test]
fn a_missing_worktree_differs_and_skips_the_checks_that_need_it() {
    let fx = Fixture::new();
    let mut run = fx.recorded();
    run.worktree = Some(fx.root.join("gone").display().to_string());
    let report = check(&run, &TestGit::default());
    assert_eq!(
        states(&report),
        vec![
            State::NotChecked,
            State::Differs,
            State::NotChecked,
            State::NotChecked,
            State::NotChecked
        ],
        "{report:#?}"
    );
    assert_eq!(
        state(&report, CheckName::Worktree).found,
        "the directory does not exist"
    );
    assert_eq!(
        state(&report, CheckName::Branch).found,
        "the recorded worktree is not available"
    );
    assert_eq!(report.exit_code, DIFFERS_EXIT);
}

#[test]
fn a_worktree_git_does_not_list_differs() {
    let fx = Fixture::new();
    // A separate repository in a directory of its own is not a worktree of `repo`.
    let stray = fx.root.join("stray");
    fs::create_dir(&stray).unwrap();
    run_git(&stray, &["init", "-q", "-b", "feature"]);
    let mut run = fx.recorded();
    run.worktree = Some(stray.display().to_string());
    let report = check(&run, &TestGit::default());
    let repository = state(&report, CheckName::Repository);
    assert_eq!(repository.state, State::Differs);
    assert!(
        repository.found.contains("the worktree's is"),
        "{}",
        repository.found
    );
    let worktree = state(&report, CheckName::Worktree);
    assert_eq!(worktree.state, State::Differs);
    assert_eq!(
        worktree.found,
        "exists, but git worktree list does not list it"
    );
}

#[test]
fn a_worktree_of_the_repository_that_git_lost_track_of_differs() {
    let fx = Fixture::new();
    // The worktree's own git file is gone: the directory is there, git does not know it.
    fs::remove_file(fx.wt.join(".git")).unwrap();
    let report = check(&fx.recorded(), &TestGit::default());
    assert_eq!(state(&report, CheckName::Worktree).state, State::Differs);
    assert_eq!(state(&report, CheckName::Branch).state, State::NotChecked);
}

#[test]
fn a_missing_repository_differs() {
    let fx = Fixture::new();
    let mut run = fx.recorded();
    run.repository = Some(fx.root.join("nowhere").display().to_string());
    let report = check(&run, &TestGit::default());
    let repository = state(&report, CheckName::Repository);
    assert_eq!(repository.state, State::Differs);
    assert_eq!(repository.found, "the directory does not exist");
    // The worktree still checks out against its own repository.
    assert_eq!(state(&report, CheckName::Worktree).state, State::Matches);
}

#[test]
fn a_directory_that_is_not_a_repository_differs() {
    let fx = Fixture::new();
    let plain = fx.root.join("plain");
    fs::create_dir(&plain).unwrap();
    let mut run = fx.recorded();
    run.repository = Some(plain.display().to_string());
    let report = check(&run, &TestGit::default());
    let repository = state(&report, CheckName::Repository);
    assert_eq!(repository.state, State::Differs);
    assert!(
        repository.found.starts_with("not a git repository"),
        "{}",
        repository.found
    );
}

#[test]
fn fields_never_recorded_are_unknown_and_do_not_fail_the_check() {
    let fx = Fixture::new();
    let run = Recorded {
        id: "7".into(),
        worktree: Some(fx.wt.display().to_string()),
        ..Recorded::default()
    };
    let report = check(&run, &TestGit::default());
    assert_eq!(
        states(&report),
        vec![
            State::Unknown,
            State::Matches,
            State::Unknown,
            State::Unknown,
            State::Matches
        ],
        "{report:#?}"
    );
    assert!(report.consistent);
    assert_eq!(report.exit_code, 0);
    assert!(render(&report).contains("never recorded"));

    let nothing = check(
        &Recorded {
            id: "8".into(),
            ..Recorded::default()
        },
        &TestGit::default(),
    );
    assert_eq!(
        states(&nothing),
        vec![
            State::Unknown,
            State::Unknown,
            State::Unknown,
            State::Unknown,
            State::Unknown
        ]
    );
    assert_eq!(
        state(&nothing, CheckName::WorkingTree).found,
        "the worktree was never recorded"
    );
    assert!(nothing.consistent);
}

#[test]
fn the_check_only_runs_reading_git_commands() {
    let fx = Fixture::new();
    fs::write(fx.wt.join("b.txt"), "changed\n").unwrap();
    let git = TestGit::default();
    check(&fx.recorded(), &git);
    let allowed = [
        "rev-parse",
        "worktree",
        "branch",
        "merge-base",
        "--no-optional-locks",
        "log",
        "for-each-ref",
    ];
    for command in git.commands.borrow().iter() {
        // Every command starts with the settings that keep repository configuration from running code.
        assert_eq!(command[..4], SAFE_CONFIG, "{command:?}");
        let command = &command[4..];
        assert!(allowed.contains(&command[0].as_str()), "{command:?}");
        if command[0] == "worktree" {
            assert_eq!(command[1], "list", "{command:?}");
        }
        if command[0] == "branch" {
            assert_eq!(command[1], "--show-current", "{command:?}");
        }
    }
}

#[test]
fn the_source_never_names_a_changing_git_command() {
    let source = include_str!("run_check.rs");
    let code = source.split("#[cfg(test)]").next().unwrap();
    for word in [
        "\"reset\"",
        "\"clean\"",
        "\"checkout\"",
        "\"switch\"",
        "\"restore\"",
        "\"fetch\"",
        "\"push\"",
        "\"commit\"",
        "\"merge\"",
        "\"stash\"",
        "\"prune\"",
        "\"remove\"",
        "\"add\"",
        "\"rm\"",
        "remove_dir",
        "remove_file",
    ] {
        assert!(!code.contains(word), "run_check.rs names {word}");
    }
}

#[test]
fn json_carries_the_states_and_lists() {
    let fx = Fixture::new();
    fs::write(fx.wt.join("x.txt"), "x\n").unwrap();
    let report = check(&fx.recorded(), &TestGit::default());
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["consistent"], false);
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["checks"][4]["check"], "working-tree");
    assert_eq!(value["checks"][4]["state"], "differs");
    assert_eq!(value["checks"][3]["check"], "base-commit");
    assert_eq!(value["checks"][3]["recorded"], fx.base);
    assert_eq!(value["changes"][0], "?? x.txt");
}

#[test]
fn control_characters_in_git_output_are_neutralized() {
    let fx = Fixture::new();
    run_git(
        &fx.wt,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "evil\u{1b}[2J subject",
        ],
    );
    let report = check(&fx.recorded(), &TestGit::default());
    assert_eq!(report.unpushed.len(), 1);
    let text = render(&report);
    assert!(!text.contains('\u{1b}'), "{text:?}");
}

#[test]
fn options_are_checked() {
    assert_eq!(parse_options(&[]), Ok(false));
    assert_eq!(parse_options(&["--json"]), Ok(true));
    assert!(parse_options(&["--json", "--json"]).is_err());
    assert!(parse_options(&["--bogus"]).is_err());
}

#[test]
fn another_valid_repository_recorded_differs() {
    let fx = Fixture::new();
    let other = fx.root.join("other");
    fs::create_dir(&other).unwrap();
    run_git(&other, &["init", "-q", "-b", "develop"]);
    let mut run = fx.recorded();
    run.repository = Some(other.display().to_string());
    let report = check(&run, &TestGit::default());
    assert_eq!(state(&report, CheckName::Repository).state, State::Differs);
    // The other repository does not list the run's worktree.
    let worktree = state(&report, CheckName::Worktree);
    assert_eq!(worktree.state, State::Differs);
    assert_eq!(
        worktree.found,
        "exists, but git worktree list does not list it"
    );
}

#[test]
fn without_remote_tracking_branches_unpushed_commits_are_not_known() {
    let fx = Fixture::new();
    run_git(&fx.repo, &["remote", "remove", "origin"]);
    let report = check(&fx.recorded(), &TestGit::default());
    assert!(report.consistent, "{report:#?}");
    assert!(report.unpushed.is_empty());
    assert_eq!(
        report.unpushed_unknown.as_deref(),
        Some("the repository has no remote-tracking branches")
    );
    assert!(render(&report).contains("Commits not pushed: not known"));
}

#[test]
fn the_fsmonitor_hook_does_not_run_during_the_check() {
    let fx = Fixture::new();
    let marker = fx.root.join("ran");
    let script = fx.root.join("hook.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    run_git(
        &fx.repo,
        &["config", "core.fsmonitor", script.to_str().unwrap()],
    );
    fs::write(fx.wt.join("b.txt"), "changed\n").unwrap();
    let report = check(&fx.recorded(), &TestGit::default());
    assert_eq!(state(&report, CheckName::WorkingTree).state, State::Differs);
    assert!(!marker.exists(), "the fsmonitor hook ran");
}
