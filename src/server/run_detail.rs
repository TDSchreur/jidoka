//! The dashboard's projection of one Run.
//!
//! It combines the stored record with its cost, the evidence recorded when its Stages ended, and the
//! current local Gate assessment.
//!
//! The run list only needs [`summary`]; Git is inspected only for [`detail`], so list polling stays
//! cheap. The browser places history rows itself from `stage_exits` and the timeline.

use crate::{
    agent::Agent,
    git::Git,
    run::gate::{self, AssessmentPurpose, ReviewSelection},
    run::inspect::{self, RouteEntry},
    run::next::{self, Reviews, StageGates},
    run::{ExitOutcome, Run, SkippedReview, Stage},
};
use serde::Serialize;

/// A run as the run list shows it, without inspecting Git.
///
/// It adds the attention indication and the estimated cost of usage to the stored record.
///
/// # Errors
///
/// Fails when the record or its cost cannot be serialized.
pub(super) fn summary(run: &Run) -> serde_json::Result<serde_json::Value> {
    let mut value = serde_json::to_value(run)?;
    value["needs_you"] = serde_json::Value::Bool(run.needs_you());
    if let Some(usage) = &run.usage {
        value["usage"] = crate::usage::cost::usage_json(usage, crate::usage::cost::PRICES)?;
    }
    Ok(value)
}

/// A run as `GET /api/runs/<id>` shows it.
///
/// It is the [`summary`] plus its model route, the skipped checks recorded when Check ended, and its
/// Stage gates and check selection as the shared local assessment sees them now. Only this projection
/// inspects the worktree through `git`.
///
/// # Errors
///
/// Fails when the projection cannot be serialized.
pub(super) fn detail(run: &Run, git: &dyn Git) -> serde_json::Result<serde_json::Value> {
    let assessment = gate::assess_local(run, git, AssessmentPurpose::Dashboard);
    let (checks, checks_unavailable) = match assessment.reviews {
        ReviewSelection::Selected { mode, selection } => (
            Some(next::reviews_for(
                mode,
                &selection,
                run.model_routing.as_ref(),
            )),
            None,
        ),
        ReviewSelection::Unavailable(reason) => (None, Some(reason)),
        ReviewSelection::NotApplicable => (None, None),
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
        stage_gates: next::stage_gates(run, assessment.evaluations),
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
    recorded_skipped_checks: Option<&'a [SkippedReview]>,
    /// Present (possibly `null`) exactly when `recorded_skipped_checks` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded_skipped_rework: Option<Option<u32>>,
    stage_gates: Vec<StageGates<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checks: Option<Reviews>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checks_unavailable: Option<String>,
}

/// What model routing requested per Stage next to what the usage shows.
#[derive(Serialize)]
struct ModelRoute {
    agent: Agent,
    stages: Vec<RouteEntry>,
}

/// The checks skipped when the run last left Check, and the Rework of that pass.
struct RecordedSkips<'a> {
    checks: &'a [SkippedReview],
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
