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
    let worktree = match run.worktree.as_deref() {
        None => None,
        Some(path) => {
            let wanted = resolved(Path::new(path));
            registered(git, repository)?
                .into_iter()
                .find(|listed| resolved(Path::new(listed)) == wanted)
                .map(|listed| (path, listed, canonical_dir(Path::new(path))))
        }
    };
    if let Some((path, _, Some(real))) = &worktree {
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
        worktree: if run.worktree.is_some() {
            Step::AlreadyGone
        } else {
            Step::NotRecorded
        },
        branch: if run.branch.is_some() {
            Step::AlreadyGone
        } else {
            Step::NotRecorded
        },
    };
    if let Some((_, listed, _)) = worktree {
        git.git(repository, &["worktree", "remove", "--", &listed])
            .map_err(|e| git_problem("worktree remove", &e))?;
        if registered(git, repository)?.contains(&listed) {
            return refuse(format!(
                "Git still lists the worktree {} after removing it (is it locked?). The branch was kept.",
                shown(&listed)
            ));
        }
        cleaned.worktree = Step::Removed;
    }
    if let Some(branch) = run.branch.as_deref().filter(|_| checked.is_some()) {
        if checked_out(git, repository, branch)? {
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

/// The worktree paths git lists, exactly as it prints them.
fn registered(git: &dyn Git, repository: &Path) -> Result<Vec<String>, ToolProblem> {
    let list = git
        .git(repository, &["worktree", "list", "--porcelain"])
        .map_err(|e| git_problem("worktree list", &e))?;
    Ok(list
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(str::to_string)
        .collect())
}

/// Whether any worktree, the main checkout included, has `branch` checked out.
fn checked_out(git: &dyn Git, repository: &Path, branch: &str) -> Result<bool, ToolProblem> {
    let list = git
        .git(repository, &["worktree", "list", "--porcelain"])
        .map_err(|e| git_problem("worktree list", &e))?;
    let wanted = format!("branch refs/heads/{branch}");
    Ok(list.lines().any(|line| line == wanted))
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
        text.push_str(&format!("Run {} is already clean.\n", run.id));
    }
    match cleaned.worktree {
        Step::Removed => text.push_str(&format!("Removed worktree {worktree}\n")),
        Step::AlreadyGone => text.push_str(&format!("Worktree {worktree} was already gone\n")),
        Step::NotRecorded => {}
    }
    match cleaned.branch {
        Step::Removed => text.push_str(&format!(
            "Deleted local branch {branch} (the remote branch is untouched)\n"
        )),
        Step::AlreadyGone => text.push_str(&format!("Local branch {branch} was already gone\n")),
        Step::NotRecorded => {}
    }
    text
}

#[cfg(test)]
#[path = "run_cleanup_tests.rs"]
mod tests;
