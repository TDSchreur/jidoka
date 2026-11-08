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
use std::path::Path;

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
            format!("{state:?}").to_lowercase()
        ));
    }
    let Some(repository) = run.repository.as_deref() else {
        return refuse("The run records no repository. Nothing was removed.");
    };
    let repository = Path::new(repository);
    if let Some(branch) = run.branch.as_deref() {
        if !valid_branch_name(branch) {
            return refuse("The recorded branch name is not valid. Nothing was removed.");
        }
    }

    // Is the worktree still registered with the repository?
    let worktree = match run.worktree.as_deref() {
        None => None,
        Some(path) => {
            let list = git
                .git(repository, &["worktree", "list", "--porcelain"])
                .map_err(|e| git_problem("worktree list", &e))?;
            let wanted = canonical_dir(Path::new(path));
            let listed = wanted.is_some()
                && list
                    .lines()
                    .filter_map(|line| line.strip_prefix("worktree "))
                    .any(|listed| canonical_dir(Path::new(listed)) == wanted);
            listed.then_some((path, wanted.unwrap_or_default()))
        }
    };
    if let Some((path, real)) = &worktree {
        if canonical_dir(cwd).is_some_and(|here| here.starts_with(real)) {
            return refuse(format!(
                "You are inside the worktree {}. Run the command from the main checkout. Nothing was removed.",
                shown(path)
            ));
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
    if let (true, Some(branch)) = (branch_exists, run.branch.as_deref()) {
        let reference = format!("refs/heads/{branch}");
        let local = git
            .git(repository, &["rev-parse", &reference])
            .map_err(|e| git_problem("rev-parse", &e))?;
        let merged = pr_state::head_commit(gh, url)?;
        if !local.eq_ignore_ascii_case(&merged) {
            return refuse(format!(
                "The local branch {} is at {local}, but the merged pull request ended at {merged}. It may hold commits that were never pushed. Nothing was removed.",
                shown(branch)
            ));
        }
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
    if let Some((path, _)) = worktree {
        git.git(repository, &["worktree", "remove", "--", path])
            .map_err(|e| git_problem("worktree remove", &e))?;
        cleaned.worktree = Step::Removed;
    }
    if let (true, Some(branch)) = (branch_exists, run.branch.as_deref()) {
        git.git(repository, &["branch", "-D", "--", branch])
            .map_err(|e| git_problem("branch -D", &e))?;
        cleaned.branch = Step::Removed;
    }
    Ok(cleaned)
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
