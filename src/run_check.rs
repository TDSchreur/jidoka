//! `factory run check`: compares what a run records with the repository, without changing anything.

use crate::{inspect::shown, pr_sync::Git, run_begin::first_line};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// What the run records about its workspace. `None` is a field that was never recorded.
#[derive(Clone, Debug, Default)]
pub(crate) struct Recorded {
    pub(crate) id: String,
    pub(crate) repository: Option<String>,
    pub(crate) worktree: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) base_commit: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CheckName {
    Repository,
    Worktree,
    Branch,
    BaseCommit,
    WorkingTree,
}

impl CheckName {
    fn label(self) -> &'static str {
        match self {
            CheckName::Repository => "repository",
            CheckName::Worktree => "worktree",
            CheckName::Branch => "branch",
            CheckName::BaseCommit => "base commit",
            CheckName::WorkingTree => "working tree",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum State {
    /// The repository agrees with the record.
    Matches,
    /// The repository does not agree; `found` says what is there.
    Differs,
    /// The field was never recorded, so there is nothing to compare.
    Unknown,
    /// The check needs the recorded worktree, which is missing or differs.
    NotChecked,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Matches => "matches",
            State::Differs => "differs",
            State::Unknown => "unknown",
            State::NotChecked => "not checked",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Check {
    pub(crate) check: CheckName,
    pub(crate) state: State,
    /// The recorded value, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recorded: Option<String>,
    /// What was found, or why the check could not run.
    pub(crate) found: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Report {
    pub(crate) run: String,
    /// True when no check differs; unknown fields do not count against it.
    pub(crate) consistent: bool,
    pub(crate) exit_code: u8,
    pub(crate) checks: Vec<Check>,
    /// Uncommitted paths in the worktree, as `git status --porcelain` prints them.
    pub(crate) changes: Vec<String>,
    /// Commits of HEAD that no remote-tracking branch has, as of the last fetch: `<hash> <subject>`.
    pub(crate) unpushed: Vec<String>,
}

/// The exit status when a check differs; 1 is left for problems.
pub(crate) const DIFFERS_EXIT: u8 = 2;

/// The most changed paths and unpushed commits that are listed.
const LIST_LIMIT: usize = 50;

/// `[--json]`.
pub(crate) fn parse_options(options: &[&str]) -> Result<bool, String> {
    let mut json = false;
    for arg in options {
        match *arg {
            "--json" if !json => json = true,
            "--json" => return Err("--json was given twice.".into()),
            other => return Err(format!("Unexpected option {other:?}.")),
        }
    }
    Ok(json)
}

/// The path with symbolic links resolved, without the `\\?\` prefix Windows adds, so two spellings of
/// one directory compare equal.
fn real(path: &Path) -> Option<PathBuf> {
    let real = path.canonicalize().ok()?;
    Some(match real.to_string_lossy().strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => real,
    })
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (real(a), real(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn git_error(error: &str) -> String {
    shown(first_line(error))
}

/// The common git directory of the repository that `dir` belongs to.
fn common_dir(git: &dyn Git, dir: &Path) -> Result<PathBuf, String> {
    let out = git.git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(PathBuf::from(out))
}

/// Up to `LIST_LIMIT` lines, with a last line saying how many more there are.
fn capped(lines: Vec<String>) -> Vec<String> {
    let total = lines.len();
    let mut list: Vec<String> = lines.into_iter().take(LIST_LIMIT).collect();
    if total > LIST_LIMIT {
        list.push(format!("... and {} more", total - LIST_LIMIT));
    }
    list
}

/// Runs the five checks of continuing a run. It only reads: no fetch, reset, clean, checkout, delete or
/// commit, and `git status` runs without taking optional locks.
pub(crate) fn check(run: &Recorded, git: &dyn Git) -> Report {
    let mut checks = Vec::new();
    let mut changes = Vec::new();
    let mut unpushed = Vec::new();
    let mut push = |check: CheckName, state: State, recorded: Option<&String>, found: String| {
        checks.push(Check {
            check,
            state,
            recorded: recorded.map(|value| shown(value)),
            found,
        });
    };

    // The worktree is the base of checks 3 to 5, so it is found first.
    let worktree = run.worktree.as_ref().map(PathBuf::from);
    let worktree_exists = worktree.as_deref().is_some_and(Path::is_dir);
    let worktree_common = worktree
        .as_deref()
        .filter(|_| worktree_exists)
        .map(|dir| common_dir(git, dir));

    // 1. The repository exists and shares its common git directory with the worktree.
    let repository = run.repository.as_ref().map(PathBuf::from);
    // Whether the recorded repository is a git repository, so the worktree list can be asked of it.
    let mut repository_is_git = false;
    match &repository {
        None => push(
            CheckName::Repository,
            State::Unknown,
            None,
            "never recorded".into(),
        ),
        Some(dir) if !dir.is_dir() => push(
            CheckName::Repository,
            State::Differs,
            run.repository.as_ref(),
            "the directory does not exist".into(),
        ),
        Some(dir) => match common_dir(git, dir) {
            Err(error) => push(
                CheckName::Repository,
                State::Differs,
                run.repository.as_ref(),
                format!("not a git repository: {}", git_error(&error)),
            ),
            Ok(common) => {
                repository_is_git = true;
                match &worktree_common {
                    Some(Ok(theirs)) if same_dir(&common, theirs) => push(
                        CheckName::Repository,
                        State::Matches,
                        run.repository.as_ref(),
                        format!(
                            "shares the git directory {} with the worktree",
                            shown(&common.display().to_string())
                        ),
                    ),
                    Some(Ok(theirs)) => push(
                        CheckName::Repository,
                        State::Differs,
                        run.repository.as_ref(),
                        format!(
                            "its git directory is {}, the worktree's is {}",
                            shown(&common.display().to_string()),
                            shown(&theirs.display().to_string())
                        ),
                    ),
                    _ => push(
                        CheckName::Repository,
                        State::Matches,
                        run.repository.as_ref(),
                        format!(
                            "a git repository ({}); there is no worktree to compare it with",
                            shown(&common.display().to_string())
                        ),
                    ),
                }
            }
        },
    }

    // 2. The worktree exists and the repository lists it.
    let mut worktree_ok = false;
    match (&worktree, &worktree_common) {
        (None, _) => push(
            CheckName::Worktree,
            State::Unknown,
            None,
            "never recorded".into(),
        ),
        (Some(_), None) => push(
            CheckName::Worktree,
            State::Differs,
            run.worktree.as_ref(),
            "the directory does not exist".into(),
        ),
        (Some(_), Some(Err(error))) => push(
            CheckName::Worktree,
            State::Differs,
            run.worktree.as_ref(),
            format!("not a git worktree: {}", git_error(error)),
        ),
        (Some(dir), Some(Ok(_))) => {
            // Ask the recorded repository when it is usable, so a worktree of another repository is not
            // found in its own list.
            let ask = match &repository {
                Some(repo) if repository_is_git => repo.as_path(),
                _ => dir.as_path(),
            };
            match git.git(ask, &["worktree", "list", "--porcelain"]) {
                Err(error) => push(
                    CheckName::Worktree,
                    State::Differs,
                    run.worktree.as_ref(),
                    format!("git worktree list failed: {}", git_error(&error)),
                ),
                Ok(list) => {
                    let listed = list
                        .lines()
                        .filter_map(|line| line.strip_prefix("worktree "))
                        .any(|path| same_dir(Path::new(path), dir));
                    if listed {
                        worktree_ok = true;
                        push(
                            CheckName::Worktree,
                            State::Matches,
                            run.worktree.as_ref(),
                            "exists and is listed by git worktree list".into(),
                        )
                    } else {
                        push(
                            CheckName::Worktree,
                            State::Differs,
                            run.worktree.as_ref(),
                            "exists, but git worktree list does not list it".into(),
                        )
                    }
                }
            }
        }
    }

    let not_checked = || {
        if worktree.is_none() {
            "the worktree was never recorded".to_string()
        } else {
            "the recorded worktree is not available".to_string()
        }
    };
    let dir = worktree.as_deref().filter(|_| worktree_ok);

    // 3. The worktree is on the recorded branch.
    match (&run.branch, dir) {
        (None, _) => push(
            CheckName::Branch,
            State::Unknown,
            None,
            "never recorded".into(),
        ),
        (Some(_), None) => push(
            CheckName::Branch,
            State::NotChecked,
            run.branch.as_ref(),
            not_checked(),
        ),
        (Some(recorded), Some(dir)) => match git.git(dir, &["branch", "--show-current"]) {
            Err(error) => push(
                CheckName::Branch,
                State::Differs,
                run.branch.as_ref(),
                format!("git branch failed: {}", git_error(&error)),
            ),
            Ok(current) if current.is_empty() => push(
                CheckName::Branch,
                State::Differs,
                run.branch.as_ref(),
                "the worktree is not on a branch (a detached HEAD)".into(),
            ),
            Ok(current) if current == *recorded => push(
                CheckName::Branch,
                State::Matches,
                run.branch.as_ref(),
                format!("on {}", shown(&current)),
            ),
            Ok(current) => push(
                CheckName::Branch,
                State::Differs,
                run.branch.as_ref(),
                format!("on {}", shown(&current)),
            ),
        },
    }

    // 4. The base commit is an ancestor of HEAD.
    match (&run.base_commit, dir) {
        (None, _) => push(
            CheckName::BaseCommit,
            State::Unknown,
            None,
            "never recorded".into(),
        ),
        (Some(_), None) => push(
            CheckName::BaseCommit,
            State::NotChecked,
            run.base_commit.as_ref(),
            not_checked(),
        ),
        (Some(base), Some(dir)) => {
            let commit = format!("{base}^{{commit}}");
            match git.git(
                dir,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &commit,
                ],
            ) {
                Err(_) => push(
                    CheckName::BaseCommit,
                    State::Differs,
                    run.base_commit.as_ref(),
                    "the commit is not in this repository".into(),
                ),
                Ok(sha) => {
                    match git.status(dir, &["merge-base", "--is-ancestor", sha.as_str(), "HEAD"]) {
                        Ok(0) => push(
                            CheckName::BaseCommit,
                            State::Matches,
                            run.base_commit.as_ref(),
                            "an ancestor of HEAD".into(),
                        ),
                        Ok(1) => push(
                            CheckName::BaseCommit,
                            State::Differs,
                            run.base_commit.as_ref(),
                            "not an ancestor of HEAD".into(),
                        ),
                        Ok(code) => push(
                            CheckName::BaseCommit,
                            State::Differs,
                            run.base_commit.as_ref(),
                            format!("git merge-base --is-ancestor failed with status {code}"),
                        ),
                        Err(error) => push(
                            CheckName::BaseCommit,
                            State::Differs,
                            run.base_commit.as_ref(),
                            format!("git merge-base failed: {}", git_error(&error)),
                        ),
                    }
                }
            }
        }
    }

    // 5. The working tree is clean; commits that are not pushed are listed.
    match dir {
        None => push(
            CheckName::WorkingTree,
            State::NotChecked,
            None,
            not_checked(),
        ),
        Some(dir) => {
            match git.git(dir, &["--no-optional-locks", "status", "--porcelain"]) {
                Err(error) => push(
                    CheckName::WorkingTree,
                    State::Differs,
                    None,
                    format!("git status failed: {}", git_error(&error)),
                ),
                Ok(status) => {
                    changes = capped(
                        status
                            .lines()
                            .map(|line| shown(line.trim_start()))
                            .collect(),
                    );
                    let count = status.lines().count();
                    if count == 0 {
                        push(
                            CheckName::WorkingTree,
                            State::Matches,
                            None,
                            "no uncommitted changes".into(),
                        )
                    } else {
                        push(
                            CheckName::WorkingTree,
                            State::Differs,
                            None,
                            format!("{count} uncommitted paths"),
                        )
                    }
                }
            }
            // A branch that was never pushed has every commit here; `--remotes` reads the last fetch.
            if let Ok(log) = git.git(
                dir,
                &[
                    "log",
                    "--no-color",
                    "--format=%h %s",
                    "HEAD",
                    "--not",
                    "--remotes",
                ],
            ) {
                unpushed = capped(log.lines().map(shown).collect());
            }
        }
    }

    let consistent = checks.iter().all(|check| check.state != State::Differs);
    Report {
        run: run.id.clone(),
        consistent,
        exit_code: if consistent { 0 } else { DIFFERS_EXIT },
        checks,
        changes,
        unpushed,
    }
}

pub(crate) fn render(report: &Report) -> String {
    let mut out = format!(
        "Run {}: {}\n",
        report.run,
        if report.consistent {
            "the recorded state matches the repository"
        } else {
            "the recorded state differs from the repository"
        }
    );
    for check in &report.checks {
        let recorded = check
            .recorded
            .as_deref()
            .map(|value| format!(" (recorded {value})"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  {:<12} {:<13} {}{}\n",
            check.state.label(),
            check.check.label(),
            check.found,
            recorded
        ));
    }
    if !report.changes.is_empty() {
        out.push_str("Uncommitted changes:\n");
        for line in &report.changes {
            out.push_str(&format!("  {line}\n"));
        }
    }
    if !report.unpushed.is_empty() {
        out.push_str("Commits not pushed (as of the last fetch):\n");
        for line in &report.unpushed {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}

#[cfg(test)]
#[path = "run_check_tests.rs"]
mod tests;
