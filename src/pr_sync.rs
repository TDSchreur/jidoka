//! `factory pr sync`: brings a run's branch up to date with its base with a normal merge commit.

use crate::{
    inspect::shown,
    pr_open::{RunApi, RunInfo},
    pr_wait::ToolProblem,
    run_begin::{first_line, git, last_line, valid_branch_name},
};
use serde::Serialize;
use std::{path::Path, process::Command};

/// Runs git. Tests override it to keep the user's own git settings out.
pub(crate) trait Git {
    /// The output of a git command that must succeed; the error is what git printed.
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        git(dir, args)
    }

    /// The exit status of a git command whose status is the answer (such as `merge-base --is-ancestor`).
    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .map_err(|error| format!("git could not be started: {error}"))?;
        output
            .status
            .code()
            .ok_or_else(|| "git was stopped by a signal".to_string())
    }
}

/// The real git.
pub(crate) struct RealGit;

impl Git for RealGit {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    /// The branch already contains the base.
    Current,
    /// The base was merged cleanly into the branch, nothing of the change overlaps, and it was pushed.
    Pushed,
    /// The base was merged cleanly but touches files of the change: not pushed until verification ran.
    NeedsVerification,
    /// The merge conflicts and is left in progress for the agent.
    Conflict,
}

impl Outcome {
    /// The exit status of the command; 1 is left for problems.
    pub(crate) fn exit_code(self) -> u8 {
        match self {
            Outcome::Current => 0,
            Outcome::Pushed => 2,
            Outcome::NeedsVerification => 3,
            Outcome::Conflict => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Synced {
    pub(crate) outcome: Outcome,
    pub(crate) exit_code: u8,
    pub(crate) branch: String,
    pub(crate) base: String,
    /// The overlapping files (needs-verification) or the conflicting files (conflict).
    pub(crate) files: Vec<String>,
    /// The merge commit, when one was made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) merge_commit: Option<String>,
    /// Commits on the branch that are not on the remote and that this command did not push.
    pub(crate) unpushed: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Options {
    pub(crate) base: Option<String>,
    pub(crate) json: bool,
}

/// `[--base <branch>] [--json]`, each at most once.
pub(crate) fn parse_options(options: &[&str]) -> Result<Options, String> {
    let mut parsed = Options::default();
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--json" if !parsed.json => parsed.json = true,
            "--base" if parsed.base.is_none() => {
                let name = args.next().copied().ok_or("--base needs a value.")?;
                if !valid_branch_name(name) {
                    return Err(format!("{name:?} is not a usable branch name for --base."));
                }
                parsed.base = Some(name.to_string());
            }
            "--json" | "--base" => return Err(format!("{arg} was given twice.")),
            other => return Err(format!("Unexpected option {other:?}.")),
        }
    }
    Ok(parsed)
}

fn lines(text: &str) -> Vec<String> {
    let mut list: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    list.sort();
    list.dedup();
    list
}

/// Brings the run's branch up to date with its base: a normal merge commit, pushed only when nothing of
/// the change overlaps. It never rebases, amends, fast-forwards, force-pushes, resets or aborts, and never
/// merges the pull request.
pub(crate) fn sync(
    run: &RunInfo,
    options: &Options,
    git: &dyn Git,
    api: &dyn RunApi,
) -> Result<Synced, ToolProblem> {
    let problem = |text: String| ToolProblem(text);
    let worktree = run.worktree.as_deref().ok_or_else(|| {
        problem(format!(
            "Run {} has no worktree recorded. Record it with `factory run update` (--worktree and --branch), or create the workspace with `factory run begin`.",
            run.id
        ))
    })?;
    let dir = Path::new(worktree);
    if !dir.is_dir() {
        return Err(problem(format!(
            "The recorded worktree {} does not exist.",
            shown(worktree)
        )));
    }
    let read = |args: &[&str], what: &str| {
        git.git(dir, args)
            .map_err(|error| problem(format!("{what} failed: {}", shown(first_line(&error)))))
    };
    let branch = read(&["branch", "--show-current"], "Reading the branch")?;
    if branch.is_empty() {
        return Err(problem(
            "The worktree is not on a branch (a detached HEAD).".into(),
        ));
    }
    if !valid_branch_name(&branch) {
        return Err(problem(format!(
            "The branch name {} is one this command will not push: it could be read as a git option or a forced refspec. Rename the branch and try again.",
            shown(&branch)
        )));
    }
    if let Some(recorded) = &run.branch {
        if *recorded != branch {
            return Err(problem(format!(
                "The worktree is on {} but the run records {}.",
                shown(&branch),
                shown(recorded)
            )));
        }
    }
    if git
        .status(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .map_err(problem)?
        == 0
    {
        return Err(problem(
            "A merge is in progress in the worktree. Resolve it (or abort it yourself), commit, and try again."
                .into(),
        ));
    }
    let base = match &options.base {
        Some(base) => base.clone(),
        None => git
            .git(
                dir,
                &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
            )
            .ok()
            .and_then(|head| head.strip_prefix("origin/").map(str::to_string))
            .ok_or_else(|| {
                problem(
                    "origin/HEAD is not set, so the base branch is not known. Pass it with --base."
                        .into(),
                )
            })?,
    };
    if branch == base {
        return Err(problem(format!(
            "{} is the base branch; there is nothing to bring up to date.",
            shown(&branch)
        )));
    }
    let status = read(&["status", "--porcelain"], "Reading the status")?;
    if !status.is_empty() {
        return Err(problem(format!(
            "The worktree has uncommitted changes ({} paths). Commit them, then try again.",
            status.lines().count()
        )));
    }

    let base_ref = format!("refs/remotes/origin/{base}");
    let refspec = format!("+refs/heads/{base}:{base_ref}");
    git.git(dir, &["fetch", "origin", &refspec])
        .map_err(|error| {
            problem(format!(
                "Fetching {} from origin failed: {}",
                shown(&base),
                shown(first_line(&error))
            ))
        })?;
    // What the remote branch is now, without touching any local ref.
    let pushed = read(
        &["ls-remote", "origin", &format!("refs/heads/{branch}")],
        "Asking origin for the branch",
    )?
    .split_whitespace()
    .next()
    .map(str::to_string);
    let push_refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let push = || {
        git.git(dir, &["push", "-u", "origin", &push_refspec])
            .map(|_| ())
            .map_err(|error| shown(last_line(&error)))
    };
    // Commits of the branch that neither the remote branch nor the base has: the agent's own unpushed work.
    let unpushed_after = |remote: &Option<String>| -> Result<u64, ToolProblem> {
        let mut args = vec!["rev-list", "--count", "HEAD", "--not", base_ref.as_str()];
        if let Some(sha) = remote {
            args.push(sha.as_str());
        }
        Ok(read(&args, "Counting unpushed commits")?
            .parse()
            .unwrap_or(0))
    };
    let synced = |outcome: Outcome,
                  files: Vec<String>,
                  merge_commit: Option<String>,
                  unpushed: u64| Synced {
        outcome,
        exit_code: outcome.exit_code(),
        branch: branch.clone(),
        base: base.clone(),
        files,
        merge_commit,
        unpushed,
    };

    match git
        .status(dir, &["merge-base", "--is-ancestor", &base_ref, "HEAD"])
        .map_err(problem)?
    {
        0 => {
            // Current. A merge of the base that an earlier run could not push is pushed now, but only when
            // HEAD is that merge: its first parent is what the remote has and its second is in the base, so
            // nothing else of the agent's goes with it.
            if let Some(remote) = &pushed {
                let parents = read(&["rev-list", "--parents", "-n", "1", "HEAD"], "Reading HEAD")?;
                let parents: Vec<&str> = parents.split_whitespace().collect();
                let merges_base = parents.len() == 3
                    && parents[1] == remote
                    && git
                        .status(dir, &["merge-base", "--is-ancestor", parents[2], &base_ref])
                        .map_err(problem)?
                        == 0;
                if merges_base {
                    push().map_err(|detail| {
                        problem(format!(
                            "Pushing the merge of {} failed: {detail}. Run the command again to push it.",
                            shown(&base)
                        ))
                    })?;
                    record(run, api, &format!("Merged {base} into the branch"));
                    return Ok(synced(Outcome::Pushed, Vec::new(), Some(parents[0].to_string()), 0));
                }
            }
            let unpushed = unpushed_after(&pushed)?;
            Ok(synced(Outcome::Current, Vec::new(), None, unpushed))
        }
        1 => {
            let merge_base = read(&["merge-base", "HEAD", &base_ref], "Finding the merge base")?;
            let own = lines(&read(
                &["diff", "--name-only", &merge_base, "HEAD"],
                "Listing the change's files",
            )?);
            let theirs = lines(&read(
                &["diff", "--name-only", &merge_base, &base_ref],
                "Listing the base's files",
            )?);
            let message = format!("Merge origin/{base} into {branch}");
            if let Err(error) = git.git(
                dir,
                &["merge", "--no-ff", "--no-edit", "-m", &message, &base_ref],
            ) {
                let conflicts = lines(&git.git(dir, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default());
                if conflicts.is_empty() {
                    return Err(problem(format!(
                        "Merging {} failed: {}",
                        shown(&base),
                        shown(last_line(&error))
                    )));
                }
                record(run, api, &format!("Merging {base}: conflicts to resolve"));
                return Ok(synced(Outcome::Conflict, conflicts, None, 0));
            }
            let merge_commit = read(&["rev-parse", "HEAD"], "Reading the merge commit")?;
            let overlap: Vec<String> = own.into_iter().filter(|file| theirs.contains(file)).collect();
            if !overlap.is_empty() {
                record(run, api, &format!("Merged {base}: verification needed"));
                return Ok(synced(Outcome::NeedsVerification, overlap, Some(merge_commit), 0));
            }
            push().map_err(|detail| {
                problem(format!(
                    "{} was merged into the branch as {}, but pushing it failed: {detail}. Run the command again to push it.",
                    shown(&base),
                    shown(&merge_commit.chars().take(7).collect::<String>())
                ))
            })?;
            record(run, api, &format!("Merged {base} into the branch"));
            Ok(synced(Outcome::Pushed, Vec::new(), Some(merge_commit), 0))
        }
        other => Err(problem(format!(
            "`git merge-base --is-ancestor` exited with {other}, so whether the branch is behind is not known. Check the repository and try again."
        ))),
    }
}

/// Notes the base update on a running run, keeping its stage and status. It is not a Rework or a CI fix.
fn record(run: &RunInfo, api: &dyn RunApi, activity: &str) {
    if run.running() {
        // The outcome stands even if the dashboard cannot be reached.
        let _ = api.update(&run.id, &run.stage, &run.status, &["--activity", activity]);
    }
}

/// The outcome and what to do next, as text.
pub(crate) fn render(synced: &Synced) -> String {
    let mut out = match synced.outcome {
        Outcome::Current => format!(
            "Current: {} contains origin/{}.\n",
            shown(&synced.branch),
            shown(&synced.base)
        ),
        Outcome::Pushed => format!(
            "Pushed: origin/{} was merged into {} and pushed. Watch CI again with `factory pr wait`.\n",
            shown(&synced.base),
            shown(&synced.branch)
        ),
        Outcome::NeedsVerification => format!(
            "Needs verification: origin/{} was merged into {}, but it touches files of the change. Rerun the project's verification on the merged code, then push.\n",
            shown(&synced.base),
            shown(&synced.branch)
        ),
        Outcome::Conflict => format!(
            "Conflict: merging origin/{} into {} conflicts. The merge is left in progress; resolve it, rerun the verification, commit and push.\n",
            shown(&synced.base),
            shown(&synced.branch)
        ),
    };
    for file in &synced.files {
        out.push_str(&format!("  {}\n", shown(file)));
    }
    if let Some(commit) = &synced.merge_commit {
        out.push_str(&format!(
            "  Merge commit: {}\n",
            shown(&commit.chars().take(7).collect::<String>())
        ));
    }
    if synced.unpushed > 0 {
        out.push_str(&format!(
            "  {} commit(s) on the branch are not pushed; this command does not push them.\n",
            synced.unpushed
        ));
    }
    out
}

#[cfg(test)]
#[path = "pr_sync_tests.rs"]
mod tests;
