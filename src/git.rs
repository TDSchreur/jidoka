//! Running git: the `Git` seam that commands and Gate evaluation take, its real implementations, and the git
//! facts several commands share (output lines, safe branch names).

use std::{
    path::Path,
    process::{Command, Output},
};

/// Runs git. Tests override it to keep the user's own git settings out.
pub(crate) trait Git {
    /// The output of a git command that must succeed; the error is what git printed.
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        run(dir, args)
    }

    /// The exit status of a git command whose status is the answer (such as `merge-base --is-ancestor`).
    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        command(dir)
            .args(args)
            .output()
            .map_err(|error| format!("git could not be started: {error}"))?
            .status
            .code()
            .ok_or_else(|| "git was stopped by a signal".to_string())
    }
}

/// The real git.
pub(crate) struct RealGit;

impl Git for RealGit {}

/// git in `dir`, with no prompts, English messages and no inherited `GIT_DIR` or `GIT_WORK_TREE`.
fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        // Its messages are told apart by callers, so they must be in English.
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    command
}

/// Runs git in `dir`. The error is what git printed, or why it could not be started.
pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = command(dir)
        .args(args)
        .output()
        .map_err(|error| format!("git could not be started: {error}"))?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Ok(if args.contains(&"-z") {
            stdout.into_owned()
        } else {
            stdout.trim().to_string()
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    Err(text.trim().to_string())
}

pub(crate) fn first_line(text: &str) -> &str {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
}

/// The last line git printed, where its `fatal:` message is (progress lines come first).
pub(crate) fn last_line(text: &str) -> &str {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
}

/// Whether `name` can be a branch name that is safe to pass to git: no option look-alike, no spaces or
/// control characters, none of the characters git refuses, and no `..`, `@{`, `.lock` ending or edge slash.
pub(crate) fn valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['-', '/', '.', '+'])
        && !name.ends_with(['/', '.'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && name != "@"
        && !name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
}

/// A worktree as `git worktree list --porcelain` prints it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Worktree {
    /// The path exactly as git prints it.
    pub(crate) path: String,
    /// The branch checked out there, if any.
    pub(crate) branch: Option<String>,
    pub(crate) locked: bool,
}

/// Every worktree git lists for `repository`, the main checkout included. The error is what git printed.
pub(crate) fn worktrees(git: &dyn Git, repository: &Path) -> Result<Vec<Worktree>, String> {
    let list = git.git(repository, &["worktree", "list", "--porcelain"])?;
    let mut entries: Vec<Worktree> = Vec::new();
    for line in list.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            entries.push(Worktree {
                path: path.to_string(),
                branch: None,
                locked: false,
            });
        } else if let Some(entry) = entries.last_mut() {
            if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                entry.branch = Some(branch.to_string());
            } else if line == "locked" || line.starts_with("locked ") {
                entry.locked = true;
            }
        }
    }
    Ok(entries)
}

/// The real git for reading: no prompts, no optional locks, and no lazy fetch of missing objects in a
/// partial clone, so nothing is written to the repository.
pub(crate) struct ReadOnlyGit;

impl ReadOnlyGit {
    fn output(dir: &Path, args: &[&str]) -> Result<Output, String> {
        command(dir)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .output()
            .map_err(|error| format!("git could not be started: {error}"))
    }
}

impl Git for ReadOnlyGit {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        let output = Self::output(dir, args)?;
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
        if output.status.success() {
            if args.contains(&"-z") {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            } else {
                Ok(text(&output.stdout))
            }
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
