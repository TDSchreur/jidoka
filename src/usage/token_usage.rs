//! The Token usage a run records: token counts per stage, model and ticket, and what is known about how
//! complete they are. Unknown usage is never shown as zero: a field a log does not report is absent.

use crate::{
    run::Stage,
    usage::pauses::{IdleGap, Pause},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Token counts. A field that was not reported is `None`, shown as `n/a`, never as zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Tokens {
    /// Input that was not read from the cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cache_write: Option<u64>,
    /// Output, including reasoning where the agent counts it there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<u64>,
    /// Of which reasoning. Part of `output`, never added to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<u64>,
}

fn sum(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
    }
}

impl Tokens {
    pub(crate) fn add(&mut self, other: &Tokens) {
        self.input = sum(self.input, other.input);
        self.cache_read = sum(self.cache_read, other.cache_read);
        self.cache_write = sum(self.cache_write, other.cache_write);
        self.output = sum(self.output, other.output);
        self.reasoning = sum(self.reasoning, other.reasoning);
    }

    pub(crate) fn is_empty(&self) -> bool {
        *self == Tokens::default()
    }

    /// Fresh tokens: input excluding cache reads, plus cache writes and output. `None` when neither
    /// input nor output was reported, so unknown is not mistaken for zero.
    pub(crate) fn fresh(&self) -> Option<u64> {
        if self.input.is_none() && self.output.is_none() {
            return None;
        }
        Some(
            self.input
                .unwrap_or(0)
                .saturating_add(self.cache_write.unwrap_or(0))
                .saturating_add(self.output.unwrap_or(0)),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UsageStatus {
    /// Every claimed entry was attributed.
    Tracked,
    /// Some usage is known; the notes say what is missing.
    Partial,
    /// Nothing is known; the reason says why.
    NotTracked,
}

/// Tokens of one stage and model, in one bucket: a ticket, the rework, or no ticket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UsageRow {
    pub(crate) stage: Stage,
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ticket: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) rework: bool,
    #[serde(flatten)]
    pub(crate) tokens: Tokens,
    /// The part of `tokens` that subagents used.
    #[serde(default, skip_serializing_if = "Tokens::is_empty")]
    pub(crate) subagents: Tokens,
    /// The part of `tokens` from requests whose total input (input, cache reads and cache writes) was above
    /// `LONG_CONTEXT_INPUT_TOKENS`, which some models bill at higher rates.
    #[serde(default, skip_serializing_if = "Tokens::is_empty")]
    pub(crate) long_context: Tokens,
    /// The reasoning efforts the agent's log recorded for these calls, sorted. Empty when it recorded none.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) efforts: BTreeSet<String>,
}

/// A request with more input than this (cache reads and writes included) is in the long-context tier.
pub(crate) const LONG_CONTEXT_INPUT_TOKENS: u64 = 272_000;

/// Most distinct efforts one row keeps. Real logs show a handful; a made-up log must not grow the record.
pub(crate) const MAX_EFFORTS_PER_ROW: usize = 8;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Usage {
    pub(crate) status: UsageStatus,
    /// Why nothing is tracked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    /// What is missing from partial usage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) notes: Vec<String>,
    /// Stages the run went through that no log has entries for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) no_data: Vec<Stage>,
    /// Milliseconds since the Unix epoch.
    pub(crate) collected_at: u64,
    /// The ids of the sessions that were counted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sessions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) rows: Vec<UsageRow>,
    /// Entries two runs could both claim, assigned to neither.
    #[serde(default, skip_serializing_if = "Tokens::is_empty")]
    pub(crate) unattributed: Tokens,
    /// Usage limits and compactions the log shows inside the run, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pauses: Vec<Pause>,
    /// Gaps of more than ten minutes between activity, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) idle: Vec<IdleGap>,
    /// Whether the logs were found and read completely, so an empty `pauses` means the log shows none.
    /// Not stored: only the collection that just ran knows.
    #[serde(skip)]
    pub(crate) log_complete: bool,
    /// Current coordinator attention state; derived from logs and never persisted.
    #[serde(skip)]
    pub(crate) attention: Option<Pause>,
    #[serde(skip)]
    pub(crate) attention_requested: bool,
    /// The skills the logs show the run used; stored as run reports, never persisted here.
    #[serde(skip)]
    pub(crate) skills: Vec<UsedSkill>,
}

/// A skill a session log shows used, with the stage the run was in at that moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsedSkill {
    pub(crate) stage: Stage,
    pub(crate) name: String,
    pub(crate) at: u64,
}

impl Usage {
    pub(crate) fn not_tracked(reason: impl Into<String>, collected_at: u64) -> Self {
        Self {
            status: UsageStatus::NotTracked,
            reason: Some(reason.into()),
            notes: Vec::new(),
            no_data: Vec::new(),
            collected_at,
            sessions: Vec::new(),
            rows: Vec::new(),
            unattributed: Tokens::default(),
            pauses: Vec::new(),
            idle: Vec::new(),
            log_complete: false,
            attention: None,
            attention_requested: false,
            skills: Vec::new(),
        }
    }

    /// Fresh tokens over every row; `None` when no row reports any.
    pub(crate) fn fresh(&self) -> Option<u64> {
        self.rows
            .iter()
            .filter_map(|row| row.tokens.fresh())
            .reduce(u64::saturating_add)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_tokens_are_input_plus_cache_writes_plus_output_and_unknown_is_not_zero() {
        let tokens = Tokens {
            input: Some(10),
            cache_read: Some(1000),
            cache_write: Some(20),
            output: Some(5),
            reasoning: Some(2),
        };
        assert_eq!(
            tokens.fresh(),
            Some(35),
            "cache reads and reasoning are not added"
        );
        assert_eq!(
            Tokens {
                cache_read: Some(5),
                ..Tokens::default()
            }
            .fresh(),
            None,
            "nothing about input or output is unknown, not 0"
        );
        let mut sum = Tokens::default();
        sum.add(&Tokens {
            input: Some(1),
            ..Tokens::default()
        });
        sum.add(&Tokens {
            input: Some(2),
            output: Some(4),
            ..Tokens::default()
        });
        assert_eq!(
            (sum.input, sum.cache_read, sum.output),
            (Some(3), None, Some(4))
        );
    }
}
