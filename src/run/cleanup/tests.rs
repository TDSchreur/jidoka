//! `jidoka run cleanup` with real git in temporary repositories and a fake pull request host.

use super::*;
use crate::git::fixture::{self, TestGit, run_git};
use crate::pr::host::{FakeHost, Head};
use std::{fs, path::PathBuf};
use tempfile::TempDir;

const PR: &str = "https://github.com/o/r/pull/7";

/// A repository `repo` on `develop` and a worktree `wt` on `feature` with one commit.
struct Fixture {
    _tmp: TempDir,
    root: PathBuf,
    repo: PathBuf,
    wt: PathBuf,
    head: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = canonical_dir(tmp.path()).unwrap();
        let repo = root.join("repo");
        let wt = root.join("wt");
        fs::create_dir(&repo).unwrap();
        run_git(&repo, &["init", "-q", "-b", "develop"]);
        fs::write(repo.join("a.txt"), "a\n").unwrap();
        run_git(&repo, &["add", "a.txt"]);
        run_git(&repo, &["commit", "-q", "-m", "base"]);
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
        let head = run_git(&wt, &["rev-parse", "HEAD"]);
        Fixture {
            _tmp: tmp,
            root,
            repo,
            wt,
            head,
        }
    }

    fn recorded(&self) -> Recorded {
        Recorded {
            id: "7".into(),
            repository: Some(self.repo.display().to_string()),
            worktree: Some(self.wt.display().to_string()),
            branch: Some("feature".into()),
            pr: Some(PR.into()),
        }
    }

    /// A host whose pull request is in `state`, from the worktree's branch at its commit.
    fn host_in(&self, state: PrState) -> FakeHost {
        FakeHost::new(vec![Ok(Head {
            state,
            commit: self.head.clone(),
            branch: "feature".into(),
        })])
    }

    /// A host whose pull request merged the worktree's branch.
    fn host(&self) -> FakeHost {
        self.host_in(PrState::Merged)
    }

    fn branch_exists(&self) -> bool {
        fixture::status(
            &self.repo,
            &["rev-parse", "--verify", "--quiet", "refs/heads/feature"],
        ) == Ok(0)
    }

    /// Cleans up from the root, which is outside the worktree.
    fn cleanup(&self, run: &Recorded, host: &FakeHost) -> Result<Cleaned, ToolProblem> {
        cleanup(run, host, &TestGit::default(), &self.root, Mode::Merged)
    }

    fn assert_untouched(&self) {
        assert!(self.wt.exists());
        assert!(self.branch_exists());
    }
}

fn refusal(result: Result<Cleaned, ToolProblem>) -> String {
    result.expect_err("cleanup should refuse").0
}

#[test]
fn a_merged_pull_request_removes_the_worktree_and_branch() {
    let fx = Fixture::new();
    let host = fx.host();
    let cleaned = fx.cleanup(&fx.recorded(), &host).unwrap();
    // State and head come from one read of the pull request.
    assert_eq!(host.calls(), [format!("head {PR}")]);
    assert_eq!(
        cleaned,
        Cleaned {
            worktree: Step::Removed,
            branch: Step::Removed
        }
    );
    assert!(!fx.wt.exists());
    assert!(!fx.branch_exists());
    assert!(cleaned.needs_update());
    let text = render(&fx.recorded(), cleaned);
    assert!(text.contains("Removed worktree"), "{text}");
    assert!(text.contains("Deleted local branch"), "{text}");
}

#[test]
fn anything_but_merged_refuses_and_removes_nothing() {
    let fx = Fixture::new();
    for state in [PrState::Open, PrState::Draft, PrState::Closed] {
        let message = refusal(fx.cleanup(&fx.recorded(), &fx.host_in(state)));
        assert!(message.contains("not merged"), "{state:?}: {message}");
        fx.assert_untouched();
    }
    let unreadable = FakeHost::new(vec![Err("The host could not be asked.".into())]);
    assert_eq!(
        refusal(fx.cleanup(&fx.recorded(), &unreadable)),
        "The host could not be asked."
    );
    fx.assert_untouched();
    let mut run = fx.recorded();
    run.pr = None;
    assert!(refusal(fx.cleanup(&run, &fx.host())).contains("no pull request"));
    fx.assert_untouched();
}

#[test]
fn uncommitted_changes_refuse() {
    let fx = Fixture::new();
    fs::write(fx.wt.join("b.txt"), "changed\n").unwrap();
    assert!(refusal(fx.cleanup(&fx.recorded(), &fx.host())).contains("uncommitted"));
    fx.assert_untouched();
}

#[test]
fn running_inside_the_worktree_refuses() {
    let fx = Fixture::new();
    let message = refusal(cleanup(
        &fx.recorded(),
        &fx.host(),
        &TestGit::default(),
        &fx.wt,
        Mode::Merged,
    ));
    assert!(message.contains("main checkout"), "{message}");
    fx.assert_untouched();
}

#[test]
fn a_rerun_after_the_worktree_was_deleted_by_hand_succeeds() {
    let fx = Fixture::new();
    fs::remove_dir_all(&fx.wt).unwrap();
    let cleaned = fx.cleanup(&fx.recorded(), &fx.host()).unwrap();
    assert_eq!(cleaned.worktree, Step::Removed);
    assert_eq!(cleaned.branch, Step::Removed);
    assert!(!fx.branch_exists());
    assert!(listed_wt(&fx).is_none());
}

/// Moves the branch just before it is deleted, like a push or commit that lands after the check.
struct MovingGit<'a>(&'a Fixture);

impl Git for MovingGit<'_> {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        if args.first() == Some(&"update-ref") {
            let moved = run_git(
                &self.0.repo,
                &[
                    "commit-tree",
                    "-p",
                    &self.0.head,
                    "-m",
                    "late",
                    "HEAD^{tree}",
                ],
            );
            run_git(&self.0.repo, &["update-ref", "refs/heads/feature", &moved]);
        }
        fixture::git(dir, args)
    }

    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        fixture::status(dir, args)
    }
}

#[test]
fn a_branch_that_moved_after_the_check_is_not_deleted() {
    let fx = Fixture::new();
    let result = cleanup(
        &fx.recorded(),
        &fx.host(),
        &MovingGit(&fx),
        &fx.root,
        Mode::Merged,
    );
    assert!(result.is_err());
    assert!(fx.branch_exists());
}

/// One entry of `git worktree list --porcelain`.
struct Listed {
    path: String,
    locked: bool,
}

fn listed(fx: &Fixture) -> Vec<Listed> {
    let mut entries: Vec<Listed> = Vec::new();
    for line in run_git(&fx.repo, &["worktree", "list", "--porcelain"]).lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            entries.push(Listed {
                path: path.to_string(),
                locked: false,
            });
        } else if line.starts_with("locked")
            && let Some(entry) = entries.last_mut()
        {
            entry.locked = true;
        }
    }
    entries
}

/// The fixture worktree's entry. Git prints its path with forward slashes and long names, so it is matched
/// by the last component of the path, which is the worktree directory's name.
fn listed_wt(fx: &Fixture) -> Option<Listed> {
    let name = fx.wt.file_name().unwrap().to_string_lossy().into_owned();
    listed(fx)
        .into_iter()
        .find(|entry| entry.path.rsplit(['/', '\\']).next() == Some(name.as_str()))
}

#[test]
fn a_locked_worktree_refuses_and_removes_nothing() {
    let fx = Fixture::new();
    run_git(&fx.repo, &["worktree", "lock", fx.wt.to_str().unwrap()]);
    let message = refusal(fx.cleanup(&fx.recorded(), &fx.host()));
    assert!(message.contains("locked"), "{message}");
    assert!(message.contains("Nothing was removed"), "{message}");
    fx.assert_untouched();
}

#[test]
fn a_branch_also_checked_out_in_the_main_checkout_refuses_before_removing() {
    let fx = Fixture::new();
    // The main checkout switches to the same branch the worktree has.
    run_git(
        &fx.repo,
        &["checkout", "-q", "--ignore-other-worktrees", "feature"],
    );
    let message = refusal(fx.cleanup(&fx.recorded(), &fx.host()));
    assert!(message.contains("checked out"), "{message}");
    assert!(message.contains("Nothing was removed"), "{message}");
    fx.assert_untouched();
}

/// Makes the feature branch known to a remote, so that no commit counts as unpushed.
fn push_feature(fx: &Fixture) {
    let remote = fx.root.join("remote.git");
    run_git(
        &fx.root,
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    run_git(
        &fx.repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&fx.repo, &["push", "-q", "origin", "feature"]);
}

fn discard(
    fx: &Fixture,
    run: &Recorded,
    host: &FakeHost,
    force: bool,
) -> Result<Cleaned, ToolProblem> {
    cleanup(
        run,
        host,
        &TestGit::default(),
        &fx.root,
        Mode::Discard { force },
    )
}

fn no_host() -> FakeHost {
    FakeHost::new(Vec::new())
}

#[test]
fn discard_removes_a_pushed_run_without_a_pull_request() {
    let fx = Fixture::new();
    push_feature(&fx);
    let mut run = fx.recorded();
    run.pr = None;
    let cleaned = discard(&fx, &run, &no_host(), false).unwrap();
    assert_eq!(cleaned.worktree, Step::Removed);
    assert_eq!(cleaned.branch, Step::Removed);
    assert!(!fx.wt.exists());
    assert!(!fx.branch_exists());
}

#[test]
fn discard_removes_a_run_whose_pull_request_was_closed() {
    let fx = Fixture::new();
    push_feature(&fx);
    discard(&fx, &fx.recorded(), &fx.host_in(PrState::Closed), false).unwrap();
    assert!(!fx.wt.exists());
    assert!(!fx.branch_exists());
}

#[test]
fn discard_refuses_a_pull_request_that_is_still_open_even_with_force() {
    let fx = Fixture::new();
    push_feature(&fx);
    for state in [PrState::Open, PrState::Draft] {
        let message = refusal(discard(&fx, &fx.recorded(), &fx.host_in(state), true));
        assert!(message.contains("Close it first"), "{state:?}: {message}");
        fx.assert_untouched();
    }
}

#[test]
fn discard_lists_unpushed_commits_and_force_removes_them() {
    let fx = Fixture::new();
    let mut run = fx.recorded();
    run.pr = None;
    let message = refusal(discard(&fx, &run, &no_host(), false));
    assert!(message.contains("no remote branch"), "{message}");
    assert!(message.contains("feature work"), "{message}");
    fx.assert_untouched();
    discard(&fx, &run, &no_host(), true).unwrap();
    assert!(!fx.wt.exists());
    assert!(!fx.branch_exists());
}

#[test]
fn discard_lists_uncommitted_changes_and_force_removes_them() {
    let fx = Fixture::new();
    push_feature(&fx);
    fs::write(fx.wt.join("scratch.txt"), "x\n").unwrap();
    let mut run = fx.recorded();
    run.pr = None;
    let message = refusal(discard(&fx, &run, &no_host(), false));
    assert!(
        message.contains("uncommitted") && message.contains("scratch.txt"),
        "{message}"
    );
    fx.assert_untouched();
    discard(&fx, &run, &no_host(), true).unwrap();
    assert!(!fx.wt.exists());
    assert!(!fx.branch_exists());
}

#[test]
fn discard_keeps_the_safety_refusals() {
    let fx = Fixture::new();
    push_feature(&fx);
    let mut run = fx.recorded();
    run.pr = None;
    let inside = cleanup(
        &run,
        &no_host(),
        &TestGit::default(),
        &fx.wt,
        Mode::Discard { force: true },
    );
    assert!(refusal(inside).contains("inside the worktree"));
    fx.assert_untouched();
}

#[test]
fn options_pick_the_mode() {
    assert_eq!(parse_options(&[]), Ok(Mode::Merged));
    assert_eq!(
        parse_options(&["--discard"]),
        Ok(Mode::Discard { force: false })
    );
    assert_eq!(
        parse_options(&["--force", "--discard"]),
        Ok(Mode::Discard { force: true })
    );
    assert!(parse_options(&["--force"]).is_err());
    assert!(parse_options(&["--nope"]).is_err());
}
