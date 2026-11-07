//! `factory run check`: compares what a run records with the repository, without changing anything.

use crate::{inspect::shown, pr_sync::Git, run_begin::first_line};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

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
    /// The same name as in `--json`.
    fn label(self) -> &'static str {
        match self {
            CheckName::Repository => "repository",
            CheckName::Worktree => "worktree",
            CheckName::Branch => "branch",
            CheckName::BaseCommit => "base-commit",
            CheckName::WorkingTree => "working-tree",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum State {
    /// The repository agrees with the record.
    Matches,
    /// The repository does not agree, or git failed there; `found` says what happened.
    Differs,
    /// The field was never recorded, so there is nothing to compare.
    Unknown,
    /// The check needs the recorded worktree, which is missing or differs.
    NotChecked,
}

impl State {
    /// The same name as in `--json`.
    fn label(self) -> &'static str {
        match self {
            State::Matches => "matches",
            State::Differs => "differs",
            State::Unknown => "unknown",
            State::NotChecked => "not-checked",
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
    /// Uncommitted paths in the worktree: the status letters and the path, as `git status --porcelain`
    /// gives them, with leading spaces removed.
    pub(crate) changes: Vec<String>,
    /// Commits of HEAD that no remote-tracking branch has, as of the last fetch: `<hash> <subject>`.
    pub(crate) unpushed: Vec<String>,
    /// Why the commits that are not pushed could not be listed, when they could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unpushed_unknown: Option<String>,
}

/// The exit status when a check differs; 1 is left for problems.
pub(crate) const DIFFERS_EXIT: u8 = 2;

/// The most changed paths and unpushed commits that are listed.
const LIST_LIMIT: usize = 50;

/// Settings put before every git command, so that the fsmonitor hook and a signature check through
/// `gpg.program` do not run. Clean and process filters that the repository sets in its attributes can
/// still run during `git status`, as they do for any `git status` there.
const SAFE_CONFIG: [&str; 4] = [
    "-c",
    "core.fsmonitor=false",
    "-c",
    "log.showSignature=false",
];

/// The real git for reading: no prompts, no optional locks, and no lazy fetch of missing objects in a
/// partial clone, so nothing is written to the repository.
pub(crate) struct ReadOnlyGit;

impl ReadOnlyGit {
    fn output(dir: &Path, args: &[&str]) -> Result<Output, String> {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("LC_ALL", "C")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .map_err(|error| format!("git could not be started: {error}"))
    }
}

impl Git for ReadOnlyGit {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        let output = Self::output(dir, args)?;
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
        Self::output(dir, args)?
            .status
            .code()
            .ok_or_else(|| "git was stopped by a signal".to_string())
    }
}

/// Runs a reading git command with the safe settings in front.
fn read(git: &dyn Git, dir: &Path, args: &[&str]) -> Result<String, String> {
    git.git(dir, &[&SAFE_CONFIG[..], args].concat())
}

/// The exit status of a reading git command, with the safe settings in front.
fn read_status(git: &dyn Git, dir: &Path, args: &[&str]) -> Result<i32, String> {
    git.status(dir, &[&SAFE_CONFIG[..], args].concat())
}

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
pub(crate) fn canonical_dir(path: &Path) -> Option<PathBuf> {
    let real = path.canonicalize().ok()?;
    Some(match real.to_string_lossy().strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => real,
    })
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (canonical_dir(a), canonical_dir(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn git_error(error: &str) -> String {
    shown(first_line(error))
}

fn display(path: &Path) -> String {
    shown(&path.display().to_string())
}

/// A full or abbreviated commit id: 4 to 64 hexadecimal digits.
fn is_commit_id(text: &str) -> bool {
    (4..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The common git directory of the repository that `dir` belongs to.
fn common_dir(git: &dyn Git, dir: &Path) -> Result<PathBuf, String> {
    read(
        git,
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
}

fn entry(check: CheckName, state: State, recorded: Option<&String>, found: String) -> Check {
    Check {
        check,
        state,
        recorded: recorded.map(|value| shown(value)),
        found,
    }
}

/// What was found at the recorded worktree path.
enum Worktree {
    NotRecorded,
    Missing,
    NotGit(String),
    Git { dir: PathBuf, common: PathBuf },
}

impl Worktree {
    fn find(git: &dyn Git, recorded: Option<&String>) -> Worktree {
        let Some(path) = recorded else {
            return Worktree::NotRecorded;
        };
        let dir = PathBuf::from(path);
        if !dir.is_dir() {
            return Worktree::Missing;
        }
        match common_dir(git, &dir) {
            Ok(common) => Worktree::Git { dir, common },
            Err(error) => Worktree::NotGit(git_error(&error)),
        }
    }
}

/// Where checks 3 to 5 run: the worktree, once it checked out.
enum Workdir<'a> {
    Usable(&'a Path),
    NotRecorded,
    Unavailable,
}

impl Workdir<'_> {
    /// The check for a recorded field that cannot be compared here, or `None` when it can.
    fn skipped(&self, check: CheckName, recorded: Option<&String>) -> Option<Check> {
        let (state, found) = match (recorded, self) {
            (None, _) => (State::Unknown, "never recorded"),
            (Some(_), Workdir::Usable(_)) => return None,
            (Some(_), Workdir::NotRecorded) => {
                (State::NotChecked, "the worktree was never recorded")
            }
            (Some(_), Workdir::Unavailable) => {
                (State::NotChecked, "the recorded worktree is not available")
            }
        };
        Some(entry(check, state, recorded, found.into()))
    }
}

/// Check 1: The repository exists and shares its common git directory with the worktree. Also returns the
/// repository's directory when it is a git repository, to ask it for its worktrees.
fn check_repository(
    git: &dyn Git,
    run: &Recorded,
    worktree: &Worktree,
) -> (Check, Option<PathBuf>) {
    let name = CheckName::Repository;
    let recorded = run.repository.as_ref();
    let Some(path) = recorded else {
        return (
            entry(name, State::Unknown, None, "never recorded".into()),
            None,
        );
    };
    let dir = PathBuf::from(path);
    if !dir.is_dir() {
        let found = "the directory does not exist".into();
        return (entry(name, State::Differs, recorded, found), None);
    }
    let common = match common_dir(git, &dir) {
        Ok(common) => common,
        Err(error) => {
            let found = format!("not a git repository: {}", git_error(&error));
            return (entry(name, State::Differs, recorded, found), None);
        }
    };
    let check = match worktree {
        Worktree::Git { common: theirs, .. } if same_dir(&common, theirs) => entry(
            name,
            State::Matches,
            recorded,
            format!(
                "shares the git directory {} with the worktree",
                display(&common)
            ),
        ),
        Worktree::Git { common: theirs, .. } => entry(
            name,
            State::Differs,
            recorded,
            format!(
                "its git directory is {}, the worktree's is {}",
                display(&common),
                display(theirs)
            ),
        ),
        Worktree::NotRecorded => entry(
            name,
            State::NotChecked,
            recorded,
            format!(
                "a git repository ({}); the worktree was never recorded",
                display(&common)
            ),
        ),
        Worktree::Missing | Worktree::NotGit(_) => entry(
            name,
            State::NotChecked,
            recorded,
            format!(
                "a git repository ({}); the recorded worktree is not available",
                display(&common)
            ),
        ),
    };
    (check, Some(dir))
}

/// Check 2: The worktree exists and is listed by the recorded repository, or by itself when the repository
/// is not usable. Returns whether the later checks can use it.
fn check_worktree(
    git: &dyn Git,
    run: &Recorded,
    worktree: &Worktree,
    repository: Option<&Path>,
) -> (Check, bool) {
    let name = CheckName::Worktree;
    let recorded = run.worktree.as_ref();
    let differs = |found: String| (entry(name, State::Differs, recorded, found), false);
    let dir = match worktree {
        Worktree::NotRecorded => {
            return (
                entry(name, State::Unknown, None, "never recorded".into()),
                false,
            )
        }
        Worktree::Missing => return differs("the directory does not exist".into()),
        Worktree::NotGit(error) => return differs(format!("not a git worktree: {error}")),
        Worktree::Git { dir, .. } => dir,
    };
    let list_from = repository.unwrap_or(dir);
    match read(git, list_from, &["worktree", "list", "--porcelain"]) {
        Err(error) => differs(format!("git worktree list failed: {}", git_error(&error))),
        Ok(list) => {
            let listed = list
                .lines()
                .filter_map(|line| line.strip_prefix("worktree "))
                .any(|path| same_dir(Path::new(path), dir));
            if listed {
                let found = "exists and is listed by git worktree list".into();
                (entry(name, State::Matches, recorded, found), true)
            } else {
                differs("exists, but git worktree list does not list it".into())
            }
        }
    }
}

/// Check 3: The worktree is on the recorded branch.
fn check_branch(git: &dyn Git, run: &Recorded, workdir: &Workdir) -> Check {
    let name = CheckName::Branch;
    let recorded = run.branch.as_ref();
    if let Some(skipped) = workdir.skipped(name, recorded) {
        return skipped;
    }
    let (Workdir::Usable(dir), Some(branch)) = (workdir, recorded) else {
        unreachable!("skipped() covers every other case");
    };
    let (state, found) = match read(git, dir, &["branch", "--show-current"]) {
        Err(error) => (
            State::Differs,
            format!("git branch failed: {}", git_error(&error)),
        ),
        Ok(current) if current.is_empty() => (
            State::Differs,
            "the worktree is not on a branch (a detached HEAD)".into(),
        ),
        Ok(current) if current == *branch => (State::Matches, format!("on {}", shown(&current))),
        Ok(current) => (State::Differs, format!("on {}", shown(&current))),
    };
    entry(name, state, recorded, found)
}

/// Check 4: The base commit is an ancestor of HEAD.
fn check_base_commit(git: &dyn Git, run: &Recorded, workdir: &Workdir) -> Check {
    let name = CheckName::BaseCommit;
    let recorded = run.base_commit.as_ref();
    if let Some(skipped) = workdir.skipped(name, recorded) {
        return skipped;
    }
    let (Workdir::Usable(dir), Some(base)) = (workdir, recorded) else {
        unreachable!("skipped() covers every other case");
    };
    if !is_commit_id(base) {
        let found = "not a commit id (4 to 64 hexadecimal digits)".into();
        return entry(name, State::Differs, recorded, found);
    }
    let commit = format!("{base}^{{commit}}");
    let Ok(sha) = read(git, dir, &["rev-parse", "--verify", "--quiet", &commit]) else {
        let found = "not found in this repository, or the abbreviation is ambiguous".into();
        return entry(name, State::Differs, recorded, found);
    };
    let (state, found) = match read_status(git, dir, &["merge-base", "--is-ancestor", &sha, "HEAD"])
    {
        Ok(0) => (State::Matches, "an ancestor of HEAD".to_string()),
        Ok(1) => (State::Differs, "not an ancestor of HEAD".to_string()),
        Ok(code) => (
            State::Differs,
            format!("git merge-base --is-ancestor failed with status {code}"),
        ),
        Err(error) => (
            State::Differs,
            format!("git merge-base failed: {}", git_error(&error)),
        ),
    };
    entry(name, state, recorded, found)
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

/// Check 5: The working tree is clean. Returns the check and the uncommitted paths.
fn check_working_tree(git: &dyn Git, workdir: &Workdir) -> (Check, Vec<String>) {
    let name = CheckName::WorkingTree;
    let dir = match workdir {
        Workdir::Usable(dir) => dir,
        Workdir::NotRecorded => {
            let found = "the worktree was never recorded".into();
            return (entry(name, State::Unknown, None, found), Vec::new());
        }
        Workdir::Unavailable => {
            let found = "the recorded worktree is not available".into();
            return (entry(name, State::NotChecked, None, found), Vec::new());
        }
    };
    match read(git, dir, &["--no-optional-locks", "status", "--porcelain"]) {
        Err(error) => {
            let found = format!("git status failed: {}", git_error(&error));
            (entry(name, State::Differs, None, found), Vec::new())
        }
        Ok(status) if status.is_empty() => (
            entry(name, State::Matches, None, "no uncommitted changes".into()),
            Vec::new(),
        ),
        Ok(status) => {
            let lines: Vec<String> = status.lines().map(|l| shown(l.trim_start())).collect();
            let found = format!("{} uncommitted paths", lines.len());
            (entry(name, State::Differs, None, found), capped(lines))
        }
    }
}

/// The commits of HEAD that no remote-tracking branch has, or why they are not known.
fn unpushed_commits(git: &dyn Git, dir: &Path) -> Result<Vec<String>, String> {
    let remotes = read(
        git,
        dir,
        &[
            "for-each-ref",
            "--count=1",
            "--format=%(refname)",
            "refs/remotes",
        ],
    )
    .map_err(|error| format!("git for-each-ref failed: {}", git_error(&error)))?;
    if remotes.is_empty() {
        return Err("the repository has no remote-tracking branches".into());
    }
    let limit = format!("--max-count={}", LIST_LIMIT + 1);
    let log = read(
        git,
        dir,
        &[
            "log",
            "--no-color",
            &limit,
            "--format=%h %s",
            "HEAD",
            "--not",
            "--remotes",
        ],
    )
    .map_err(|error| format!("git log failed: {}", git_error(&error)))?;
    let mut list: Vec<String> = log.lines().map(shown).collect();
    if list.len() > LIST_LIMIT {
        list.truncate(LIST_LIMIT);
        list.push("... and more".into());
    }
    Ok(list)
}

/// Runs the five checks of continuing a run. It only reads: no fetch, reset, clean, checkout, delete or
/// commit. `git status` takes no optional locks, and the fsmonitor hook and signature checks are off.
pub(crate) fn check(run: &Recorded, git: &dyn Git) -> Report {
    let worktree = Worktree::find(git, run.worktree.as_ref());
    let (repository, repository_dir) = check_repository(git, run, &worktree);
    let (worktree_check, usable) = check_worktree(git, run, &worktree, repository_dir.as_deref());
    let workdir = match (&worktree, usable) {
        (Worktree::Git { dir, .. }, true) => Workdir::Usable(dir),
        (Worktree::NotRecorded, _) => Workdir::NotRecorded,
        _ => Workdir::Unavailable,
    };
    let branch = check_branch(git, run, &workdir);
    let base_commit = check_base_commit(git, run, &workdir);
    let (working_tree, changes) = check_working_tree(git, &workdir);
    let (unpushed, unpushed_unknown) = match &workdir {
        Workdir::Usable(dir) => match unpushed_commits(git, dir) {
            Ok(list) => (list, None),
            Err(why) => (Vec::new(), Some(why)),
        },
        _ => (Vec::new(), None),
    };

    let checks = vec![
        repository,
        worktree_check,
        branch,
        base_commit,
        working_tree,
    ];
    let consistent = checks.iter().all(|check| check.state != State::Differs);
    Report {
        run: run.id.clone(),
        consistent,
        exit_code: if consistent { 0 } else { DIFFERS_EXIT },
        checks,
        changes,
        unpushed,
        unpushed_unknown,
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
    if let Some(why) = &report.unpushed_unknown {
        out.push_str(&format!("Commits not pushed: not known, {why}\n"));
    }
    out
}

#[cfg(test)]
#[path = "run_check_tests.rs"]
mod tests;
