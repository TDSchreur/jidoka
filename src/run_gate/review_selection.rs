//! Changed-file Review selection: which Reviews apply to the paths a run changed since its base commit.

use super::local::AssessmentPurpose;
use crate::{
    git::Git,
    reviews::{self, Selection},
    run::{Mode, Run, RunStatus, SkippedReview, Stage},
};
use std::path::Path;

/// Which Reviews apply to the run's diff, when the Check stage needs to know.
pub(crate) enum ReviewSelection {
    NotApplicable,
    Unavailable(String),
    Selected { mode: Mode, selection: Selection },
}

/// The Reviews to show or require for `purpose`.
///
/// In the Check stage they come from the diff; outside it, `purpose` decides whether to preview them.
pub(super) fn for_assessment(
    run: &Run,
    git: &dyn Git,
    purpose: AssessmentPurpose,
) -> ReviewSelection {
    let mode = match purpose {
        AssessmentPurpose::Dashboard => match run.mode {
            None => {
                return ReviewSelection::Unavailable(
                    "Record the workflow mode to select checks".into(),
                );
            }
            Some(Mode::Fast) => {
                return ReviewSelection::Selected {
                    mode: Mode::Fast,
                    selection: reviews::select(Mode::Fast, &[] as &[String]),
                };
            }
            Some(mode) => Some(mode),
        },
        AssessmentPurpose::Guidance
            if run.stage == Stage::Check && run.status == RunStatus::Done =>
        {
            return match run.mode.filter(|&mode| mode != Mode::Fast) {
                Some(mode) => ReviewSelection::Selected {
                    mode,
                    selection: reviews::select(mode, &[] as &[String]),
                },
                None => ReviewSelection::NotApplicable,
            };
        }
        AssessmentPurpose::Advance if run.status != RunStatus::Running => None,
        AssessmentPurpose::Advance | AssessmentPurpose::Guidance
            if run.stage == Stage::Check && run.status != RunStatus::Done =>
        {
            run.mode.filter(|&mode| mode != Mode::Fast)
        }
        AssessmentPurpose::Advance | AssessmentPurpose::Guidance => None,
    };
    let Some(mode) = mode else {
        return ReviewSelection::NotApplicable;
    };
    match select(run, mode, git) {
        Ok(selection) => ReviewSelection::Selected { mode, selection },
        Err(reason) => ReviewSelection::Unavailable(reason),
    }
}

/// The Reviews that apply to the paths changed since the run's base commit.
pub(super) fn select(run: &Run, mode: Mode, git: &dyn Git) -> Result<Selection, String> {
    changed_paths(run, git).map(|paths| reviews::select(mode, &paths))
}

fn changed_paths(run: &Run, git: &dyn Git) -> Result<Vec<String>, String> {
    let worktree = run
        .worktree
        .as_ref()
        .ok_or("Record the worktree to select checks")?;
    let base = run
        .base_commit
        .as_ref()
        .ok_or("Record the base commit to select checks")?;
    let range = format!("{base}..HEAD");
    git.git(
        Path::new(worktree),
        &["diff", "--name-only", "--no-renames", "-z", &range],
    )
    .map(|out| out.split_terminator('\0').map(str::to_string).collect())
    .map_err(|error| format!("Could not inspect the changed paths in {worktree}: {error}"))
}

/// The Reviews `selection` skipped, as a Stage exit records them.
pub(super) fn skipped_snapshot(selection: &Selection) -> Vec<SkippedReview> {
    selection
        .skipped
        .iter()
        .map(|entry| SkippedReview {
            name: entry.review.name.to_owned(),
            reason: entry.reason.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Exit;
    use crate::run_gate::tests::run;

    #[test]
    fn skipped_snapshot_survives_serialization_and_rework() {
        static EXTRA: crate::reviews::Review = crate::reviews::Review {
            name: "extra",
            triggers: &["*.tf"],
            instructions: "extra",
            gate: crate::reviews::ReviewGate::Recorded,
            model_route: None,
            floor: &[],
        };
        let selection = crate::reviews::Selection {
            applicable: vec![],
            skipped: vec![crate::reviews::Skipped {
                review: &EXTRA,
                reason: "no matching path".into(),
            }],
        };
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        run.leave(
            Exit::Accepted {
                gates: Vec::new(),
                skipped_checks: skipped_snapshot(&selection),
                ci: None,
            },
            0,
        )
        .unwrap();
        let mut restored: Run =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        restored.rework = Some(1);
        assert_eq!(restored.stage_exits[0].skipped_checks[0].name, "extra");
        assert_eq!(
            restored.stage_exits[0].skipped_checks[0].reason,
            "no matching path"
        );
    }

    struct NullDiff;

    impl Git for NullDiff {
        fn git(&self, _dir: &Path, args: &[&str]) -> Result<String, String> {
            assert_eq!(
                args,
                ["diff", "--name-only", "--no-renames", "-z", "abc..HEAD"]
            );
            Ok(" leading.txt\0line\nbreak.tf\0ümlaut.tf\0".into())
        }
    }

    #[test]
    fn changed_paths_preserves_unusual_names() {
        let mut run = run(Stage::Check);
        run.worktree = Some("/work/repo".into());
        run.base_commit = Some("abc".into());
        assert_eq!(
            changed_paths(&run, &NullDiff).unwrap(),
            [" leading.txt", "line\nbreak.tf", "ümlaut.tf"]
        );
    }

    #[test]
    fn git_diff_preserves_unusual_paths_and_both_sides_of_renames() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| crate::git::fixture::run_git(dir.path(), args);
        git(&["init", "-q"]);
        let commit = |message| git(&["commit", "-q", "--allow-empty", "-m", message]);
        std::fs::write(dir.path().join("security.tf"), "resource").unwrap();
        git(&["add", "--all"]);
        commit("base");
        let base = git(&["rev-parse", "HEAD"]);
        git(&["mv", "security.tf", "renamed.txt"]);
        std::fs::write(dir.path().join(" leading.txt"), "one").unwrap();
        std::fs::write(dir.path().join("ümlaut.tf"), "two").unwrap();
        git(&["add", "--all"]);
        commit("change");
        let mut run = run(Stage::Check);
        run.worktree = Some(dir.path().to_string_lossy().into_owned());
        run.base_commit = Some(base);
        let paths = changed_paths(&run, &crate::git::ReadOnlyGit).unwrap();
        assert!(paths.contains(&" leading.txt".to_string()));
        assert!(paths.contains(&"ümlaut.tf".to_string()));
        assert!(paths.contains(&"security.tf".to_string()));
        assert!(paths.contains(&"renamed.txt".to_string()));
    }
}
