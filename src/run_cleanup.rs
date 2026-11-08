//! `factory run cleanup`: removes the worktree and local branch of a Run whose pull request merged, and only
//! when no work would be lost. The remote branch is never touched.

use crate::{
    inspect::shown,
    pr_state::{self, PrState},
    pr_sync::Git,
    pr_wait::{Gh, ToolProblem},
    run_begin::{first_line, valid_branch_name},
    run_check::canonical_dir,
};
use std::path::{Path, PathBuf};

/// What the Run records about its workspace.
#[derive(Clone, Debug, Default)]
pub(crate) struct Recorded {
    pub(crate) id: String,
    pub(crate) repository: Option<String>,
    pub(crate) worktree: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) pr: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Removed,
    /// Nothing was left to remove.
    AlreadyGone,
    /// The path exists but git does not list it as a worktree, so it was not touched.
    LeftAlone,
    /// The Run no longer records it.
    NotRecorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cleaned {
    pub(crate) worktree: Step,
    pub(crate) branch: Step,
}

impl Cleaned {
    /// Whether the Run still records something, so its context has to be cleared.
    pub(crate) fn needs_update(self) -> bool {
        self.worktree != Step::NotRecorded || self.branch != Step::NotRecorded
    }
}

fn refuse<T>(message: impl Into<String>) -> Result<T, ToolProblem> {
    Err(ToolProblem(message.into()))
}

fn git_problem(what: &str, error: &str) -> ToolProblem {
    ToolProblem(format!("`git {what}` failed: {}", shown(first_line(error))))
}

/// Removes the Run's worktree and local branch when its pull request is merged and nothing would be lost.
/// Every refusal comes before anything is removed. `cwd` is where the command runs.
pub(crate) fn cleanup(
    run: &Recorded,
    gh: &dyn Gh,
    git: &dyn Git,
    cwd: &Path,
) -> Result<Cleaned, ToolProblem> {
    if run.worktree.is_none() && run.branch.is_none() {
        return Ok(Cleaned {
            worktree: Step::NotRecorded,
            branch: Step::NotRecorded,
        });
    }
    let Some(url) = run.pr.as_deref() else {
        return refuse(
            "The run has no pull request recorded, so nothing shows its work is merged. Nothing was removed.",
        );
    };
    let state = pr_state::read(gh, url)?;
    if state != PrState::Merged {
        return refuse(format!(
            "The pull request is not merged (it is {}). Nothing was removed.",
            state.label()
        ));
    }
    if run.worktree.is_some() && run.branch.is_none() {
        return refuse(
            "The run records a worktree but no branch, so nothing ties that worktree to the pull request. Nothing was removed.",
        );
    }
    let Some(repository) = run.repository.as_deref() else {
        return refuse("The run records no repository. Nothing was removed.");
    };
    let repository = Path::new(repository);
    if repository.is_relative()
        || run
            .worktree
            .as_deref()
            .is_some_and(|w| Path::new(w).is_relative())
    {
        return refuse(
            "The recorded repository or worktree path is relative, so it cannot be told apart from the current directory. Nothing was removed.",
        );
    }
    // The pull request must come from the branch the run recorded, or its merge says nothing about that branch.
    let head = match run.branch.as_deref() {
        None => None,
        Some(branch) => {
            if !valid_branch_name(branch) {
                return refuse("The recorded branch name is not valid. Nothing was removed.");
            }
            let head = pr_state::head(gh, url)?;
            if head.branch != branch {
                return refuse(format!(
                    "The pull request comes from branch {}, not the run's branch {}. Nothing was removed.",
                    shown(&head.branch),
                    shown(branch)
                ));
            }
            Some(head)
        }
    };

    // Is the worktree still registered with the repository? `listed` is the path as git printed it.
    let entries = worktrees(git, repository)?;
    let own = match run.worktree.as_deref() {
        None => None,
        Some(path) => {
            let wanted = resolved(Path::new(path));
            entries
                .iter()
                .find(|entry| resolved(Path::new(&entry.path)) == wanted)
        }
    };
    if let Some(entry) = own.filter(|entry| entry.locked) {
        return refuse(format!(
            "The worktree {} is locked. Unlock it yourself first. Nothing was removed.",
            shown(&entry.path)
        ));
    }
    if let Some(branch) = run.branch.as_deref() {
        if entries
            .iter()
            .filter(|entry| Some(*entry) != own)
            .any(|entry| entry.branch.as_deref() == Some(branch))
        {
            return refuse(format!(
                "The branch {} is checked out in another worktree (the main checkout included). Nothing was removed.",
                shown(branch)
            ));
        }
    }
    let worktree = run
        .worktree
        .as_deref()
        .zip(own)
        .map(|(path, entry)| Target {
            path,
            listed: entry.path.clone(),
            real: canonical_dir(Path::new(path)),
        });
    if let Some(Target {
        path,
        real: Some(real),
        ..
    }) = &worktree
    {
        if canonical_dir(cwd).is_some_and(|here| here.starts_with(real)) {
            return refuse(format!(
                "You are inside the worktree {}. Run the command from the main checkout. Nothing was removed.",
                shown(path)
            ));
        }
        if let Some(branch) = run.branch.as_deref() {
            let on = git.git(Path::new(path), &["symbolic-ref", "-q", "HEAD"]);
            if on.ok().as_deref() != Some(format!("refs/heads/{branch}").as_str()) {
                return refuse(format!(
                    "The worktree {} is not on the recorded branch {} (it is detached or on another branch). Nothing was removed.",
                    shown(path),
                    shown(branch)
                ));
            }
        }
        let status = git
            .git(Path::new(path), &["status", "--porcelain"])
            .map_err(|e| git_problem("status", &e))?;
        if !status.is_empty() {
            return refuse(format!(
                "The worktree has uncommitted changes. Commit or discard them yourself. Nothing was removed.\n{}",
                shown(&status)
            ));
        }
    }

    // The branch may only go when it holds nothing the merged pull request lacks.
    let branch_exists = match run.branch.as_deref() {
        None => false,
        Some(branch) => {
            let reference = format!("refs/heads/{branch}");
            git.status(
                repository,
                &["rev-parse", "--verify", "--quiet", &reference],
            )
            .map_err(|e| git_problem("rev-parse", &e))?
                == 0
        }
    };
    let mut checked = None;
    if let (true, Some(branch), Some(head)) = (branch_exists, run.branch.as_deref(), &head) {
        let reference = format!("refs/heads/{branch}");
        let local = git
            .git(repository, &["rev-parse", &reference])
            .map_err(|e| git_problem("rev-parse", &e))?;
        let merged = &head.commit;
        if !local.eq_ignore_ascii_case(merged) {
            return refuse(format!(
                "The local branch {} is at {local}, but the merged pull request ended at {merged}. It may hold commits that were never pushed. Nothing was removed.",
                shown(branch)
            ));
        }
        checked = Some(local);
    }

    let mut cleaned = Cleaned {
        worktree: match run.worktree.as_deref() {
            None => Step::NotRecorded,
            // A directory git does not list is not ours to remove.
            Some(path) if worktree.is_none() && Path::new(path).exists() => Step::LeftAlone,
            Some(_) => Step::AlreadyGone,
        },
        branch: if run.branch.is_some() {
            Step::AlreadyGone
        } else {
            Step::NotRecorded
        },
    };
    if let Some(Target { listed, .. }) = worktree {
        git.git(repository, &["worktree", "remove", "--", &listed])
            .map_err(|e| git_problem("worktree remove", &e))?;
        // The pre-checks already ran, so this re-list only guards against a race since then.
        if worktrees(git, repository)?
            .iter()
            .any(|entry| entry.path == listed)
        {
            return refuse(format!(
                "Git still lists the worktree {} after removing it (is it locked?). The branch was kept.",
                shown(&listed)
            ));
        }
        cleaned.worktree = Step::Removed;
    }
    // The pre-checks already ran, so this re-check only guards against a race since then.
    if let Some(branch) = run.branch.as_deref().filter(|_| checked.is_some()) {
        if worktrees(git, repository)?
            .iter()
            .any(|entry| entry.branch.as_deref() == Some(branch))
        {
            return refuse(format!(
                "The branch {} is still checked out in a worktree. The branch was kept.",
                shown(branch)
            ));
        }
    }
    if let (Some(oid), Some(branch)) = (checked, run.branch.as_deref()) {
        // Deleting only at the checked commit keeps a branch that moved since the check.
        let reference = format!("refs/heads/{branch}");
        git.git(repository, &["update-ref", "-d", &reference, &oid])
            .map_err(|e| git_problem("update-ref -d", &e))?;
        cleaned.branch = Step::Removed;
    }
    Ok(cleaned)
}

/// A worktree as `git worktree list --porcelain` prints it.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    /// The path exactly as git prints it.
    path: String,
    /// The branch checked out there, if any.
    branch: Option<String>,
    locked: bool,
}

/// The run's worktree as git lists it.
struct Target<'a> {
    /// The path the run recorded.
    path: &'a str,
    /// The path as git printed it.
    listed: String,
    /// The path on disk, when it exists.
    real: Option<PathBuf>,
}

/// Every worktree git lists, the main checkout included.
fn worktrees(git: &dyn Git, repository: &Path) -> Result<Vec<Entry>, ToolProblem> {
    let list = git
        .git(repository, &["worktree", "list", "--porcelain"])
        .map_err(|e| git_problem("worktree list", &e))?;
    let mut entries: Vec<Entry> = Vec::new();
    for line in list.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            entries.push(Entry {
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

/// `path` with its nearest existing ancestor canonicalised and the missing components appended, so a path
/// that no longer exists still compares equal to its listed form behind a symlinked parent.
fn resolved(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut here = path;
    loop {
        if let Some(real) = canonical_dir(here) {
            return missing.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (here.parent(), here.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                here = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

pub(crate) fn render(run: &Recorded, cleaned: Cleaned) -> String {
    let shown_or = |value: &Option<String>| value.as_deref().map(shown).unwrap_or_default();
    let worktree = shown_or(&run.worktree);
    let branch = shown_or(&run.branch);
    let mut text = String::new();
    if cleaned.worktree != Step::Removed && cleaned.branch != Step::Removed {
        if cleaned.worktree == Step::LeftAlone {
            text.push_str("Nothing was removed.\n");
        } else {
            text.push_str(&format!("Run {} is already clean.\n", run.id));
        }
    }
    match cleaned.worktree {
        Step::Removed => text.push_str(&format!("Removed worktree {worktree}\n")),
        Step::AlreadyGone => text.push_str(&format!("Worktree {worktree} was already gone\n")),
        Step::LeftAlone => text.push_str(&format!(
            "Worktree {worktree} exists but git does not list it as a worktree, so it was left alone (not removed)\n"
        )),
        Step::NotRecorded => {}
    }
    match cleaned.branch {
        Step::Removed => text.push_str(&format!(
            "Deleted local branch {branch} (the remote branch is untouched)\n"
        )),
        Step::AlreadyGone => text.push_str(&format!("Local branch {branch} was already gone\n")),
        Step::LeftAlone | Step::NotRecorded => {}
    }
    text
}

#[cfg(test)]
#[path = "run_cleanup_tests.rs"]
mod tests;
