//! `jidoka run check` with real git in temporary repositories and a local bare remote.

use super::*;
use crate::git::fixture::{TestGit, real_path, run_git};
use std::fs;
use tempfile::TempDir;

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
