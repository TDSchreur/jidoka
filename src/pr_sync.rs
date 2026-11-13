//! `jidoka pr sync`: brings a run's branch up to date with its base with a normal merge commit.

use crate::{
    inspect::shown,
    pr_open::{PrRun, RunApi},
    pr_state::PrState,
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
    /// Whether the branch exists on origin.
    pub(crate) on_origin: bool,
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

/// What to do next after a refusal because the pull request is over.
fn cleanup_hint(run: &PrRun, state: PrState) -> String {
    if state == PrState::Merged {
        format!(" Run `jidoka run cleanup {}`.", run.id)
    } else {
        String::new()
    }
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
#[expect(
    clippy::too_many_lines,
    reason = "validates the worktree, then fetches, rebases and pushes and reports each step"
)]
pub(crate) fn sync(
    run: &PrRun,
    options: &Options,
    git: &dyn Git,
    api: &dyn RunApi,
) -> Result<Synced, ToolProblem> {
    let problem = |text: String| ToolProblem(text);
    let worktree = run.worktree.as_deref().ok_or_else(|| {
        problem(format!(
            "Run {} has no worktree recorded. Record it with `jidoka run update` (--worktree and --branch), or create the workspace with `jidoka run begin`.",
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
    if let Some(state @ (PrState::Merged | PrState::Closed)) = run.pr_state {
        return Err(problem(format!(
            "The pull request is {}, so nothing was merged or pushed.{}",
            state.label(),
            cleanup_hint(run, state)
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
            "The branch name {} is one this command will not push: it could be read as a git option or a forced refspec. Rename the branch (`git branch -m`) and try again.",
            shown(&branch)
        )));
    }
    if let Some(recorded) = &run.branch
        && *recorded != branch
    {
        return Err(problem(format!(
            "The worktree is on {} but the run records {}. Switch it back, or record the right branch with `jidoka run update`.",
            shown(&branch),
            shown(recorded)
        )));
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
    git.git(
        dir,
        &["fetch", "origin", &format!("+refs/heads/{base}:{base_ref}")],
    )
    .map_err(|error| {
        problem(format!(
            "Fetching {} from origin failed: {}",
            shown(&base),
            shown(first_line(&error))
        ))
    })?;
    // A branch with commits of its own that the base already contains was merged (also by a merge commit
    // the stored state does not know yet); merging the base again would only make an empty merge commit.
    if let Some(start) = &run.base_commit
        && git
            .git(dir, &["rev-list", "--count", &format!("{start}..HEAD")])
            .is_ok_and(|count| count != "0")
        && git
            .status(dir, &["merge-base", "--is-ancestor", "HEAD", &base_ref])
            .map_err(problem)?
            == 0
    {
        return Err(problem(format!(
            "The branch is already in origin/{base}: the pull request was merged, so nothing was merged or pushed.{}",
            cleanup_hint(run, PrState::Merged)
        )));
    }
    // What the remote branch is now. It is fetched too, so that its commits are known here.
    let remote = read(
        &["ls-remote", "origin", &format!("refs/heads/{branch}")],
        "Asking origin for the branch",
    )?
    .split_whitespace()
    .next()
    .map(str::to_string);
    if remote.is_some() {
        let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        git.git(dir, &["fetch", "origin", &refspec])
            .map_err(|error| {
                problem(format!(
                    "Fetching {} from origin failed: {}",
                    shown(&branch),
                    shown(first_line(&error))
                ))
            })?;
    }
    if let Some(sha) = &remote
        && git
            .status(dir, &["merge-base", "--is-ancestor", sha, "HEAD"])
            .map_err(problem)?
            != 0
    {
        return Err(problem(format!(
            "origin's {} has commits you do not have (someone pushed to it). Merge origin/{} into the branch, rerun the verification, then try again.",
            shown(&branch),
            shown(&branch)
        )));
    }
    // Commits of the branch that neither the remote branch nor the base has: work that is not pushed.
    let count_unpushed = || -> Result<u64, ToolProblem> {
        let mut args = vec!["rev-list", "--count", "HEAD", "--not", base_ref.as_str()];
        if let Some(sha) = &remote {
            args.push(sha.as_str());
        }
        let text = read(&args, "Counting unpushed commits")?;
        text.parse().map_err(|_| {
            problem(format!(
                "`git rev-list --count` gave {}, not a number.",
                shown(&text)
            ))
        })
    };
    let files_between = |from: &str, to: &str, what: &str| -> Result<Vec<String>, ToolProblem> {
        // Without rename detection a renamed file shows under both names, so a rename still overlaps.
        Ok(lines(&read(
            &["diff", "--no-renames", "--name-only", from, to],
            what,
        )?))
    };
    let push_refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let push = || {
        git.git(dir, &["push", "-u", "origin", &push_refspec])
            .map(|_| ())
            .map_err(|error| {
                let lower = error.to_lowercase();
                // `[remote rejected]` is a hook or a branch rule saying no; only these mean the branch moved on.
                if lower.contains("non-fast-forward") || lower.contains("fetch first") || lower.contains("[rejected]") {
                    format!(
                        "origin rejected it: the remote branch moved on. Run the command again; it says what the remote has. ({})",
                        shown(last_line(&error))
                    )
                } else {
                    // git puts its reason on the `!` line; the last line only says that the push failed.
                    let reason = error
                        .lines()
                        .map(str::trim)
                        .find(|line| line.starts_with('!'))
                        .unwrap_or_else(|| last_line(&error));
                    format!(
                        "{}. Fix what it says, then run the command again to push it.",
                        shown(reason)
                    )
                }
            })
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
        on_origin: remote.is_some(),
    };

    match git
        .status(dir, &["merge-base", "--is-ancestor", &base_ref, "HEAD"])
        .map_err(problem)?
    {
        0 => {
            // Current. A merge of the base whose push failed earlier is pushed now, but only when HEAD is
            // that merge exactly as git makes it without help (no conflict was resolved by hand), on top of
            // what the remote has, and the base touched none of the change's files: the cases that never
            // needed verification.
            if let Some(sha) = &remote {
                let parents = read(
                    &["rev-list", "--parents", "-n", "1", "HEAD"],
                    "Reading HEAD",
                )?;
                let parents: Vec<&str> = parents.split_whitespace().collect();
                if parents.len() == 3
                    && parents[1] == sha
                    && git
                        .status(dir, &["merge-base", "--is-ancestor", parents[2], &base_ref])
                        .map_err(problem)?
                        == 0
                {
                    let (ours, theirs) = (parents[1], parents[2]);
                    let start = read(&["merge-base", ours, theirs], "Finding the merge base")?;
                    let own = files_between(&start, ours, "Listing the change's files")?;
                    let base_files = files_between(&start, theirs, "Listing the base's files")?;
                    let untouched = own.iter().all(|file| !base_files.contains(file));
                    // 0 is a clean merge, 1 a conflict (resolved by hand here); anything else is a git that
                    // cannot answer, such as one before 2.38.
                    let automatic = match git
                        .status(dir, &["merge-tree", "--write-tree", ours, theirs])
                        .map_err(problem)?
                    {
                        0 => {
                            let tree = read(
                                &["merge-tree", "--write-tree", ours, theirs],
                                "Merging in memory",
                            )?;
                            first_line(&tree)
                                == read(&["rev-parse", "HEAD^{tree}"], "Reading HEAD")?
                        }
                        1 => false,
                        _ => {
                            return Err(problem(format!(
                                "HEAD is a merge of {} that is not pushed, and this git cannot check whether it is the plain merge (`git merge-tree --write-tree` needs git 2.38 or later). Verify it and push it by hand, or update git.",
                                shown(&base)
                            )));
                        }
                    };
                    if untouched && automatic {
                        push().map_err(|detail| {
                            problem(format!(
                                "Pushing the merge of {} failed: {detail}",
                                shown(&base)
                            ))
                        })?;
                        record(run, api, &format!("Merged {base} into the branch"));
                        return Ok(synced(
                            Outcome::Pushed,
                            Vec::new(),
                            Some(parents[0].to_string()),
                            0,
                        ));
                    }
                }
            }
            Ok(synced(
                Outcome::Current,
                Vec::new(),
                None,
                count_unpushed()?,
            ))
        }
        1 => {
            let unpushed = count_unpushed()?;
            let merge_base = read(&["merge-base", "HEAD", &base_ref], "Finding the merge base")?;
            let own = files_between(&merge_base, "HEAD", "Listing the change's files")?;
            let theirs = files_between(&merge_base, &base_ref, "Listing the base's files")?;
            let message = format!("Merge origin/{base} into {branch}");
            if let Err(error) = git.git(
                dir,
                &["merge", "--no-ff", "--no-edit", "-m", &message, &base_ref],
            ) {
                let conflicts = lines(
                    &git.git(dir, &["diff", "--name-only", "--diff-filter=U"])
                        .unwrap_or_default(),
                );
                if conflicts.is_empty() {
                    return Err(problem(format!(
                        "Merging {} failed: {}",
                        shown(&base),
                        shown(last_line(&error))
                    )));
                }
                record(run, api, &format!("Merging {base}: conflicts to resolve"));
                return Ok(synced(Outcome::Conflict, conflicts, None, unpushed));
            }
            let merge_commit = read(&["rev-parse", "HEAD"], "Reading the merge commit")?;
            let overlap: Vec<String> = own
                .into_iter()
                .filter(|file| theirs.contains(file))
                .collect();
            // Only a merge on top of exactly what the remote has, touching nothing of the change, is pushed:
            // commits that are not pushed yet may not be verified.
            if !overlap.is_empty() || unpushed > 0 || remote.is_none() {
                record(run, api, &format!("Merged {base}: verification needed"));
                return Ok(synced(
                    Outcome::NeedsVerification,
                    overlap,
                    Some(merge_commit),
                    unpushed,
                ));
            }
            push().map_err(|detail| {
                problem(format!(
                    "{} was merged into the branch as {}, but pushing it failed: {detail}",
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
fn record(run: &PrRun, api: &dyn RunApi, activity: &str) {
    if run.running() {
        // The outcome stands even if the dashboard cannot be reached.
        let _ = api.update(&run.id, &run.status, &["--activity", activity]);
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
            "Pushed: origin/{} was merged into {} and pushed. Watch CI again with `jidoka pr wait`.\n",
            shown(&synced.base),
            shown(&synced.branch)
        ),
        Outcome::NeedsVerification => format!(
            "Needs verification: origin/{} was merged into {} but not pushed. Rerun the project's verification on the merged code, then push.\n{}",
            shown(&synced.base),
            shown(&synced.branch),
            if synced.files.is_empty() {
                ""
            } else {
                "  The base touched these files of the change:\n"
            }
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
    if synced.outcome == Outcome::NeedsVerification && !synced.on_origin {
        out.push_str("  The branch is not on origin yet.\n");
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
