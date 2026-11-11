//! `jidoka run cleanup` with real git in temporary repositories and a scripted `gh`.

use super::*;
use crate::pr_wait::{GhError, GhOutput};
use std::{fs, path::PathBuf, process::Command};
use tempfile::TempDir;

const PR: &str = "https://github.com/o/r/pull/7";

/// Git without the user's own settings, with a fixed author so that commits can be made.
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

struct TestGit;

impl Git for TestGit {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        let out = output(dir, args);
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
        if out.status.success() {
            Ok(text(&out.stdout))
        } else {
            Err(text(&out.stderr))
        }
    }

    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        output(dir, args)
            .status
            .code()
            .ok_or_else(|| "signal".to_string())
    }
}

/// Answers the state and head commit questions about the pull request.
struct FakeGh {
    state: &'static str,
    draft: bool,
    head: String,
    /// The pull request's head branch.
    branch: String,
}

impl Gh for FakeGh {
    fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        let stdout = if args.iter().any(|a| a.contains("headRefOid")) {
            format!(
                r#"{{"headRefOid":"{}","headRefName":"{}"}}"#,
                self.head, self.branch
            )
        } else {
            format!(r#"{{"state":"{}","isDraft":{}}}"#, self.state, self.draft)
        };
        Ok(GhOutput {
            code: Some(0),
            stdout,
            stderr: String::new(),
        })
    }
}

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

    fn gh(&self) -> FakeGh {
        FakeGh {
            state: "MERGED",
            draft: false,
            head: self.head.clone(),
            branch: "feature".into(),
        }
    }

    fn branch_exists(&self) -> bool {
        output(
            &self.repo,
            &["rev-parse", "--verify", "--quiet", "refs/heads/feature"],
        )
        .status
        .success()
    }

    /// Cleans up from the root, which is outside the worktree.
    fn cleanup(&self, run: &Recorded, gh: &FakeGh) -> Result<Cleaned, ToolProblem> {
        cleanup(run, gh, &TestGit, &self.root)
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
    let cleaned = fx.cleanup(&fx.recorded(), &fx.gh()).unwrap();
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
    for (state, draft) in [
        ("OPEN", false),
        ("OPEN", true),
        ("CLOSED", false),
        ("WEIRD", false),
    ] {
        let gh = FakeGh {
            state,
            draft,
            head: fx.head.clone(),
            branch: "feature".into(),
        };
        let message = refusal(fx.cleanup(&fx.recorded(), &gh));
        assert!(!message.is_empty(), "{state}");
        fx.assert_untouched();
    }
    let mut run = fx.recorded();
    run.pr = None;
    assert!(refusal(fx.cleanup(&run, &fx.gh())).contains("no pull request"));
    fx.assert_untouched();
}

#[test]
fn uncommitted_changes_refuse() {
    let fx = Fixture::new();
    fs::write(fx.wt.join("b.txt"), "changed\n").unwrap();
    assert!(refusal(fx.cleanup(&fx.recorded(), &fx.gh())).contains("uncommitted"));
    fx.assert_untouched();
}

#[test]
fn running_inside_the_worktree_refuses() {
    let fx = Fixture::new();
    let message = refusal(cleanup(&fx.recorded(), &fx.gh(), &TestGit, &fx.wt));
    assert!(message.contains("main checkout"), "{message}");
    fx.assert_untouched();
}

#[test]
fn a_rerun_after_the_worktree_was_deleted_by_hand_succeeds() {
    let fx = Fixture::new();
    fs::remove_dir_all(&fx.wt).unwrap();
    let cleaned = fx.cleanup(&fx.recorded(), &fx.gh()).unwrap();
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
        TestGit.git(dir, args)
    }

    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        TestGit.status(dir, args)
    }
}

#[test]
fn a_branch_that_moved_after_the_check_is_not_deleted() {
    let fx = Fixture::new();
    let result = cleanup(&fx.recorded(), &fx.gh(), &MovingGit(&fx), &fx.root);
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
    let message = refusal(fx.cleanup(&fx.recorded(), &fx.gh()));
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
    let message = refusal(fx.cleanup(&fx.recorded(), &fx.gh()));
    assert!(message.contains("checked out"), "{message}");
    assert!(message.contains("Nothing was removed"), "{message}");
    fx.assert_untouched();
}
