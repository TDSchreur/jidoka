//! The dashboard's view of one Run: the stored record with its cost, the evidence recorded when its
//! Stages ended, and the current local Gate assessment.
//!
//! The run list only needs [`summary`]; Git is inspected only for [`detail`], so list polling stays
//! cheap. The browser places history rows itself from `stage_exits` and the timeline.

use crate::{
    git::Git,
    inspect::{self, RouteEntry},
    routing::RoutingAgent,
    run::{ExitOutcome, Run, SkippedCheck, Stage},
    run_gate::{self, AssessmentPurpose, CheckSelection},
    run_next::{self, Checks, StageGates},
};
use serde::Serialize;

/// A run as the run list shows it: the stored record plus the estimated cost of its usage, from the
/// price table as it is now. The cost is never stored.
pub(super) fn summary(run: &Run) -> serde_json::Result<serde_json::Value> {
    let mut value = serde_json::to_value(run)?;
    if let Some(usage) = &run.usage {
        value["usage"] = crate::cost::usage_json(usage, crate::cost::PRICES)?;
    }
    Ok(value)
}

/// A run as `GET /api/runs/<id>` shows it: the [`summary`] plus its model route, the skipped checks
/// recorded when Check ended, and its Stage gates and check selection as the shared local assessment
/// sees them now. Only this projection inspects the worktree through `git`.
pub(super) fn detail(run: &Run, git: &dyn Git) -> serde_json::Result<serde_json::Value> {
    let assessment = run_gate::assess_local(run, git, AssessmentPurpose::Dashboard);
    let (checks, checks_unavailable) = match assessment.checks {
        CheckSelection::Selected { mode, selection } => (
            Some(run_next::checks_for(
                mode,
                &selection,
                run.model_routing.as_ref(),
            )),
            None,
        ),
        CheckSelection::Unavailable(reason) => (None, Some(reason)),
        CheckSelection::NotApplicable => (None, None),
    };
    let recorded = RecordedSkips::of(run);
    serde_json::to_value(Detail {
        run: summary(run)?,
        model_route: run.model_routing.as_ref().map(|routing| ModelRoute {
            agent: routing.agent,
            stages: inspect::route_entries(routing, run.usage.as_ref(), run.rework),
        }),
        recorded_skipped_checks: recorded.as_ref().map(|skips| skips.checks),
        recorded_skipped_rework: recorded.map(|skips| skips.rework),
        stage_gates: run_next::stage_gates(run, assessment.evaluations),
        checks,
        checks_unavailable,
    })
}

/// The fields the detail adds to the [`summary`]. They come after the record, so they win over a stored
/// field of the same name.
#[derive(Serialize)]
struct Detail<'a> {
    #[serde(flatten)]
    run: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_route: Option<ModelRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded_skipped_checks: Option<&'a [SkippedCheck]>,
    /// Present (possibly `null`) exactly when `recorded_skipped_checks` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded_skipped_rework: Option<Option<u32>>,
    stage_gates: Vec<StageGates<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checks: Option<Checks>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checks_unavailable: Option<String>,
}

/// What model routing requested per Stage next to what the usage shows.
#[derive(Serialize)]
struct ModelRoute {
    agent: RoutingAgent,
    stages: Vec<RouteEntry>,
}

/// The checks skipped when the run last left Check, and the Rework of that pass.
struct RecordedSkips<'a> {
    checks: &'a [SkippedCheck],
    rework: Option<u32>,
}

impl<'a> RecordedSkips<'a> {
    /// The latest accepted Check exit wins. A run recorded before Stage exits existed falls back to
    /// its latest gate result from a check.
    fn of(run: &'a Run) -> Option<Self> {
        let exit = run
            .stage_exits
            .iter()
            .rev()
            .find(|exit| exit.stage == Stage::Check && exit.outcome == ExitOutcome::Accepted);
        if let Some(exit) = exit {
            return Some(Self {
                checks: &exit.skipped_checks,
                rework: exit.rework,
            });
        }
        run.gates
            .iter()
            .rev()
            .find(|result| result.check.is_some())
            .map(|result| Self {
                checks: &result.skipped_checks,
                rework: result.rework,
            })
    }
}
