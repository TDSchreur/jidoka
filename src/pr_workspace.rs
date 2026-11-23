//! The Workspace that `jidoka pr open` and `jidoka pr sync` work in: the run facts they need, the one operation
//! that verifies the Workspace and selects the base before anything is merged or pushed, the push itself, and
//! what they report back to the run.

use crate::{
    git::{Git, first_line, valid_branch_name},
    inspect::shown,
    pr_host::ToolProblem,
    pr_state::PrState,
    run::{FieldUpdate, Run, UpdateRun},
};
use std::path::{Path, PathBuf};

/// What `pr open` and `pr sync` need to know about the run.
#[derive(Clone, Debug)]
pub(crate) struct PrRun {
    pub(crate) id: String,
    pub(crate) worktree: Option<String>,
    pub(crate) branch: Option<String>,
    /// `running`, `blocked` or `done`. Only a running run is updated.
    pub(crate) status: String,
    /// The commit the branch started from, when recorded. `pr sync` uses it to tell the branch's own commits.
    pub(crate) base_commit: Option<String>,
    /// The pull request state the dashboard last stored. `pr sync` reads it instead of asking the host.
    pub(crate) pr_state: Option<PrState>,
}

impl PrRun {
    /// The facts of run `id` as the Jidoka server stores it.
    pub(crate) fn from_run(id: &str, run: &Run) -> Self {
        let status = serde_json::to_value(run.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "running".to_string());
        Self {
            id: id.to_string(),
            worktree: run.worktree.clone(),
            branch: run.branch.clone(),
            status,
            base_commit: run.base_commit.clone(),
            pr_state: run.pr_status.as_ref().map(|status| status.state),
        }
    }

    pub(crate) fn running(&self) -> bool {
        self.status == "running"
    }
}

/// A run's Workspace that passed the checks `pr open` and `pr sync` share.
pub(crate) struct Workspace {
    pub(crate) dir: PathBuf,
    pub(crate) branch: String,
    /// The base branch, from `--base` or origin's default branch.
    pub(crate) base: String,
}

impl Workspace {
    /// Checks the run's Workspace before anything is merged or pushed: the worktree exists, it is on a branch that
    /// is safe to push and that the run records, no merge is in progress, the base is another branch, and nothing
    /// is uncommitted. `base` is the `--base` the user gave; without it, origin's default branch is selected.
    pub(crate) fn verify(
        run: &PrRun,
        base: Option<&str>,
        git: &dyn Git,
    ) -> Result<Self, ToolProblem> {
        let worktree = run.worktree.as_deref().ok_or_else(|| {
        ToolProblem(format!(
            "Run {} has no worktree recorded. Record it with `jidoka run update` (--worktree and --branch), or create the workspace with `jidoka run begin`.",
            run.id
        ))
    })?;
        let dir = Path::new(worktree);
        if !dir.is_dir() {
            return Err(ToolProblem(format!(
                "The recorded worktree {} does not exist.",
                shown(worktree)
            )));
        }
        let branch = git
            .git(dir, &["branch", "--show-current"])
            .map_err(|error| {
                ToolProblem(format!(
                    "{} could not be read: {}",
                    shown(worktree),
                    shown(first_line(&error))
                ))
            })?;
        if branch.is_empty() {
            return Err(ToolProblem(format!(
                "The worktree {} is not on a branch (a detached HEAD).",
                shown(worktree)
            )));
        }
        // A branch name that git could read as an option or a refspec (`+main` forces an update of main) is
        // never handed to git.
        if !valid_branch_name(&branch) {
            return Err(ToolProblem(format!(
                "The branch name {} is one this command will not push: it could be read as a git option or a forced refspec. Rename the branch (`git branch -m`) and try again.",
                shown(&branch)
            )));
        }
        if let Some(recorded) = &run.branch
            && *recorded != branch
        {
            return Err(ToolProblem(format!(
                "The worktree is on {} but the run records {}. Switch it back, or record the right branch with `jidoka run update`.",
                shown(&branch),
                shown(recorded)
            )));
        }
        if git
            .status(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .map_err(ToolProblem)?
            == 0
        {
            return Err(ToolProblem(
            "A merge is in progress in the worktree. Resolve it (or abort it yourself), commit, and try again."
                .into(),
        ));
        }
        let base = match base {
            Some(base) => base.to_string(),
            None => git
                .git(
                    dir,
                    &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
                )
                .ok()
                .and_then(|head| head.strip_prefix("origin/").map(str::to_string))
                .ok_or_else(|| {
                    ToolProblem(
                    "origin/HEAD is not set, so the base branch is not known. Pass it with --base."
                        .into(),
                )
                })?,
        };
        if branch == base {
            return Err(ToolProblem(format!(
                "{} is the base branch. A pull request needs a feature branch.",
                shown(&branch)
            )));
        }
        let status = git.git(dir, &["status", "--porcelain"]).map_err(|error| {
            ToolProblem(format!(
                "The status of {} could not be read: {}",
                shown(worktree),
                shown(first_line(&error))
            ))
        })?;
        if !status.is_empty() {
            return Err(ToolProblem(format!(
                "The worktree has uncommitted changes ({} paths). Commit them, then try again.",
                status.lines().count()
            )));
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            branch,
            base,
        })
    }
}

/// Pushes `branch` to origin and sets it as the upstream. It never forces: the branch is pushed by its full
/// name on both sides, so that no name is read as a refspec. The error is what git printed.
pub(crate) fn push_branch(git: &dyn Git, dir: &Path, branch: &str) -> Result<(), String> {
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    git.git(dir, &["push", "-u", "origin", &refspec])
        .map(|_| ())
}

/// What `pr open` and `pr sync` tell a running run. Each is an update the server already takes on
/// `POST /api/runs/{id}`; none changes the run's stage or status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PrReport {
    /// The pull request is open at this URL; the run records it.
    Opened { pull_request: String },
    /// The base was merged into the branch and pushed.
    BaseMerged { base: String },
    /// The base was merged into the branch but needs the project's verification before it is pushed.
    BaseNeedsVerification { base: String },
    /// Merging the base conflicts; the merge is left in progress.
    BaseConflicts { base: String },
}

impl PrReport {
    /// The activity the run shows.
    pub(crate) fn activity(&self) -> String {
        match self {
            Self::Opened { .. } => "Opened pull request".to_string(),
            Self::BaseMerged { base } => format!("Merged {base} into the branch"),
            Self::BaseNeedsVerification { base } => format!("Merged {base}: verification needed"),
            Self::BaseConflicts { base } => format!("Merging {base}: conflicts to resolve"),
        }
    }

    /// The run update that carries this report.
    pub(crate) fn update(&self) -> UpdateRun {
        let pr = match self {
            Self::Opened { pull_request } => FieldUpdate::Set(pull_request.clone()),
            _ => FieldUpdate::Keep,
        };
        UpdateRun {
            activity: FieldUpdate::Set(self.activity()),
            pr,
            ..UpdateRun::default()
        }
    }
}

/// Where `pr open` and `pr sync` send their reports: the Jidoka server, or a fake in tests.
pub(crate) trait RunReporter {
    fn report(&self, id: &str, report: &PrReport) -> Result<(), String>;
}

#[cfg(test)]
#[path = "pr_workspace_tests.rs"]
mod tests;
