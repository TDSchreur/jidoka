//! The Stage graph per Workflow mode and the Gates that guard each exit, as data.
//!
//! The graph only says what may happen next and which Gates must hold first; recording and checking
//! Gate results is done elsewhere.

// The commands that use this module arrive in later tickets of the stage machine.
#![allow(
    dead_code,
    reason = "used by `run next` and `run advance`, which are added later"
)]

use crate::run::{Mode, RunStatus, Stage};
use serde::{Deserialize, Serialize};

/// A condition that must hold before a run may leave a Stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Gate {
    /// Every ticket is `done`.
    TicketsDone,
    /// The full verification passed on the head commit.
    Verified,
    /// Verification passed after each ticket, not only after the last one.
    TicketsVerified,
    /// Every applicable check has a recorded result.
    ChecksRecorded,
    /// The recorded checks left no actionable findings; otherwise the run starts a Rework.
    NoActionableFindings,
    /// CI passed on the pull request's head commit, or the project has no CI.
    CiPassed,
}

/// Where a run goes when it leaves a Stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Destination {
    Stage(Stage),
    /// The run is finished; its status becomes `done`.
    Done,
}

impl Destination {
    /// The run status this destination sets, if it sets one.
    pub(crate) fn status(self) -> Option<RunStatus> {
        match self {
            Self::Stage(_) => None,
            Self::Done => Some(RunStatus::Done),
        }
    }
}

/// A legal move out of a Stage and the Gates that must pass before it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Transition {
    pub(crate) to: Destination,
    pub(crate) gates: &'static [Gate],
}

/// The Stages a mode goes through, in order.
pub(crate) fn stages(mode: Mode) -> &'static [Stage] {
    match mode {
        Mode::Fast => &[Stage::Define, Stage::Build, Stage::Finish],
        Mode::Balanced | Mode::Full => &[Stage::Define, Stage::Build, Stage::Check, Stage::Finish],
    }
}

/// Where a run in `stage` goes next, and the Gates that must pass first.
///
/// # Errors
///
/// Returns a message when the run is blocked or done, or when `stage` is not part of `mode`.
pub(crate) fn next(mode: Mode, stage: Stage, status: RunStatus) -> Result<Transition, String> {
    ensure_running(status)?;
    let order = stages(mode);
    let index = position(mode, stage)?;
    let to = order
        .get(index + 1)
        .map_or(Destination::Done, |next| Destination::Stage(*next));
    Ok(Transition {
        to,
        gates: exit_gates(mode, stage),
    })
}

/// The move from Check back to Build to fix actionable findings.
///
/// The cap on Reworks is a policy of the workflow and is not enforced here.
///
/// # Errors
///
/// Returns a message when the run is blocked or done, or when `stage` is not Check in a mode that has one.
pub(crate) fn rework(mode: Mode, stage: Stage, status: RunStatus) -> Result<Transition, String> {
    ensure_running(status)?;
    position(mode, stage)?;
    if stage != Stage::Check {
        return Err(format!("Rework starts from check, not {}", stage.name()));
    }
    Ok(Transition {
        to: Destination::Stage(Stage::Build),
        gates: &[],
    })
}

fn ensure_running(status: RunStatus) -> Result<(), String> {
    match status {
        RunStatus::Running => Ok(()),
        RunStatus::Blocked => {
            Err("the run is blocked; resume it before changing stage".to_string())
        }
        RunStatus::Done => Err("the run is done".to_string()),
    }
}

fn position(mode: Mode, stage: Stage) -> Result<usize, String> {
    stages(mode)
        .iter()
        .position(|candidate| *candidate == stage)
        .ok_or_else(|| format!("{} has no {} stage", mode.name(), stage.name()))
}

fn exit_gates(mode: Mode, stage: Stage) -> &'static [Gate] {
    match (mode, stage) {
        (_, Stage::Define) => &[],
        (Mode::Fast, Stage::Build) => &[Gate::TicketsDone, Gate::Verified],
        (Mode::Balanced | Mode::Full, Stage::Build) => &[Gate::TicketsDone],
        (_, Stage::Check) => &[
            Gate::ChecksRecorded,
            Gate::NoActionableFindings,
            Gate::Verified,
        ],
        (_, Stage::Finish) => &[Gate::CiPassed],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: RunStatus = RunStatus::Running;

    fn walk(mode: Mode) -> Vec<Stage> {
        let mut path = vec![Stage::Define];
        while let Destination::Stage(next_stage) =
            next(mode, *path.last().unwrap(), RUNNING).unwrap().to
        {
            path.push(next_stage);
        }
        path
    }

    #[test]
    fn each_mode_walks_its_own_stages_and_ends_in_done() {
        assert_eq!(
            walk(Mode::Fast),
            [Stage::Define, Stage::Build, Stage::Finish]
        );
        for mode in [Mode::Balanced, Mode::Full] {
            assert_eq!(
                walk(mode),
                [Stage::Define, Stage::Build, Stage::Check, Stage::Finish]
            );
        }
        for mode in [Mode::Fast, Mode::Balanced, Mode::Full] {
            let last = *stages(mode).last().unwrap();
            let end = next(mode, last, RUNNING).unwrap();
            assert_eq!(end.to, Destination::Done);
            assert_eq!(end.to.status(), Some(RunStatus::Done));
        }
    }

    #[test]
    fn fast_has_no_check_stage() {
        let error = next(Mode::Fast, Stage::Check, RUNNING).unwrap_err();
        assert!(error.contains("jidoka-fast has no check stage"), "{error}");
        assert!(rework(Mode::Fast, Stage::Check, RUNNING).is_err());
    }

    #[test]
    fn build_gates_differ_by_mode() {
        let gates = |mode| next(mode, Stage::Build, RUNNING).unwrap().gates;
        assert_eq!(gates(Mode::Fast), [Gate::TicketsDone, Gate::Verified]);
        assert_eq!(gates(Mode::Balanced), [Gate::TicketsDone]);
        assert_eq!(gates(Mode::Full), [Gate::TicketsDone]);
    }

    #[test]
    fn check_and_finish_are_guarded() {
        for mode in [Mode::Balanced, Mode::Full] {
            let check = next(mode, Stage::Check, RUNNING).unwrap();
            assert_eq!(check.to, Destination::Stage(Stage::Finish));
            assert_eq!(
                check.gates,
                [
                    Gate::ChecksRecorded,
                    Gate::NoActionableFindings,
                    Gate::Verified
                ]
            );
        }
        for mode in [Mode::Fast, Mode::Balanced, Mode::Full] {
            assert_eq!(
                next(mode, Stage::Finish, RUNNING).unwrap().gates,
                [Gate::CiPassed]
            );
            assert!(next(mode, Stage::Define, RUNNING).unwrap().gates.is_empty());
        }
    }

    #[test]
    fn rework_returns_from_check_to_build() {
        for mode in [Mode::Balanced, Mode::Full] {
            let back = rework(mode, Stage::Check, RUNNING).unwrap();
            assert_eq!(back.to, Destination::Stage(Stage::Build));
        }
    }

    #[test]
    fn rework_is_illegal_outside_check() {
        for stage in [Stage::Define, Stage::Build, Stage::Finish] {
            let error = rework(Mode::Full, stage, RUNNING).unwrap_err();
            assert!(error.contains("Rework starts from check"), "{error}");
        }
    }

    #[test]
    fn blocked_and_done_runs_cannot_change_stage() {
        for status in [RunStatus::Blocked, RunStatus::Done] {
            assert!(next(Mode::Full, Stage::Build, status).is_err());
            assert!(rework(Mode::Full, Stage::Check, status).is_err());
        }
    }
}
