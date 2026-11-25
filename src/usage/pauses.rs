//! Pauses and idle gaps of a run, derived from the pause markers and activity in the session logs it claims.

use crate::{
    run::Run,
    usage::{agent_logs::LogSession, session_attribution::FUTURE_SLACK_MS},
};
use serde::{Deserialize, Serialize};

/// What made a run stop working for a while.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PauseKind {
    UsageLimit,
    Compaction,
    AgentStopped,
}

impl PauseKind {
    pub(crate) const ALL: [Self; 3] = [Self::UsageLimit, Self::Compaction, Self::AgentStopped];

    /// The pause kind whose blocking reason this is: `None` for a reason the agent gave.
    pub(crate) fn from_reason(reason: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.reason() == reason)
    }

    /// The blocking reason set while this pause lasts.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::UsageLimit => "Usage limit reached",
            Self::Compaction => "Context compaction in progress",
            Self::AgentStopped => "Agent stopped; it may need you",
        }
    }
}

/// A marker in the agent's log that work stopped, and when the next activity followed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Pause {
    pub(crate) kind: PauseKind,
    /// Milliseconds since the Unix epoch.
    pub(crate) at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resumed_at: Option<u64>,
}

/// A stretch with no activity in the agent's log that no pause explains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IdleGap {
    pub(crate) from: u64,
    pub(crate) to: u64,
}

/// Activity further apart than this is an idle gap.
pub(crate) const IDLE_GAP_MS: u64 = 10 * 60 * 1000;
/// Most pauses and most idle gaps one run keeps (the cap applies to each list), so a long or made-up log
/// cannot grow the record. Pauses keep the newest, dropping the oldest; idle gaps keep the longest, in
/// time order.
const MAX_PAUSES: usize = 50;

/// The pauses and the idle gaps of `target` in the sessions it claims, inside its window. A gap that a
/// pause explains is not also an idle gap.
pub(crate) fn derive_pauses(
    target: &Run,
    sessions: &[LogSession],
    now: u64,
) -> (Vec<Pause>, Vec<IdleGap>) {
    let Some(first) = target.timeline.first() else {
        return (Vec::new(), Vec::new());
    };
    let (start, end) = (
        first.at,
        target
            .finished_at
            .unwrap_or(now.saturating_add(FUTURE_SLACK_MS)),
    );
    let inside = |at: &u64| (start..=end).contains(at);
    let claimed = sessions.iter().filter(|session| session.belongs_to(target));
    let mut markers: Vec<(PauseKind, u64)> = Vec::new();
    let mut activity: Vec<u64> = Vec::new();
    for session in claimed {
        markers.extend(session.pauses.iter().filter(|(_, at)| inside(at)));
        activity.extend(session.activity.iter().filter(|at| inside(at)));
    }
    markers.sort_by_key(|(_, at)| *at);
    activity.sort_unstable();
    activity.dedup();
    let mut pauses: Vec<Pause> = Vec::new();
    for (kind, at) in markers {
        let resumed_at = activity.iter().copied().find(|ts| *ts > at);
        // A repeat of a marker that nothing separates from the last one is the same pause.
        if pauses
            .last()
            .is_some_and(|last| last.kind == kind && last.resumed_at == resumed_at)
        {
            continue;
        }
        pauses.push(Pause {
            kind,
            at,
            resumed_at,
        });
    }
    let mut idle: Vec<IdleGap> = activity
        .windows(2)
        .filter(|pair| pair[1] - pair[0] > IDLE_GAP_MS)
        .map(|pair| IdleGap {
            from: pair[0],
            to: pair[1],
        })
        .filter(|gap| !pauses.iter().any(|p| (gap.from..=gap.to).contains(&p.at)))
        .collect();
    if idle.len() > MAX_PAUSES {
        // Keep the longest ones.
        idle.sort_by_key(|gap| std::cmp::Reverse(gap.to - gap.from));
        idle.truncate(MAX_PAUSES);
        idle.sort_by_key(|gap| gap.from);
    }
    pauses.drain(..pauses.len().saturating_sub(MAX_PAUSES));
    (pauses, idle)
}
