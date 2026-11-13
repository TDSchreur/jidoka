//! Token usage of a run, read from the agents' session logs and kept as counts only.
//!
//! A log entry is attributed to the run's stage, model and ticket by the run's own timeline of marks. A
//! session belongs to a run only when its id was recorded on the run or its log names the run id; never by
//! folder or time alone. An entry that two runs could both claim is not assigned to either. Unknown usage is
//! never shown as zero: a field a log does not report is absent.

use crate::{
    run::{Run, Stage},
    usage_logs::{ReadLimits, Roots, RunSession, read_sessions},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    time::Instant,
};

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

/// One moment the run's stage, rework or active ticket changed. Usage entries are attributed to the last
/// mark at or before their timestamp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mark {
    /// Milliseconds since the Unix epoch.
    pub(crate) at: u64,
    pub(crate) stage: Stage,
    /// The rework count while Build is working on a rework.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rework: Option<u32>,
    /// The reference of the ticket in progress, in Build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ticket: Option<String>,
    /// What the run was doing when it started this pass, from the update that created the mark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<String>,
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

/// What made a run stop working for a while.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PauseKind {
    UsageLimit,
    Compaction,
}

impl PauseKind {
    pub(crate) const ALL: [Self; 2] = [Self::UsageLimit, Self::Compaction];

    /// The pause kind whose blocking reason this is: `None` for a reason the agent gave.
    pub(crate) fn from_reason(reason: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.reason() == reason)
    }

    /// The blocking reason set while this pause lasts.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::UsageLimit => "Usage limit reached",
            Self::Compaction => "Context compaction in progress",
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

// --- time -------------------------------------------------------------------------------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Milliseconds since the epoch for an RFC 3339 timestamp such as `2026-11-05T21:17:49.123Z` or with a
/// `+01:00` offset; `None` for anything else.
pub(crate) fn parse_timestamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let digits = |from: usize, len: usize| -> Option<i64> {
        let part = bytes.get(from..from + len)?;
        part.iter().all(u8::is_ascii_digit).then(|| {
            part.iter()
                .fold(0i64, |acc, digit| acc * 10 + i64::from(digit - b'0'))
        })
    };
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut at = 19;
    let mut millis = 0;
    if bytes.get(at) == Some(&b'.') {
        let fraction_start = at + 1;
        at = fraction_start;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let fraction = &text[fraction_start..at];
        if fraction.is_empty() {
            return None;
        }
        let padded = format!("{:0<3}", &fraction[..fraction.len().min(3)]);
        millis = padded.parse::<i64>().ok()?;
    }
    let offset_minutes = match bytes.get(at)? {
        b'Z' | b'z' if at + 1 == bytes.len() => 0,
        sign @ (b'+' | b'-') if at + 6 == bytes.len() && bytes[at + 3] == b':' => {
            let minutes = digits(at + 1, 2)? * 60 + digits(at + 4, 2)?;
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => return None,
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
        - offset_minutes * 60;
    u64::try_from(seconds * 1000 + millis).ok()
}

/// `2026-11-05 21:17 UTC` for milliseconds since the epoch.
pub(crate) fn format_timestamp(millis: u64) -> String {
    let seconds = (millis / 1000) as i64;
    let (y, m, d) = civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        in_day / 3600,
        in_day % 3600 / 60
    )
}

/// `2026-11-05T10:00:05.000Z`, for fixtures that need a log timestamp.
#[cfg(test)]
pub(crate) fn iso_timestamp(millis: u64) -> String {
    let seconds = millis / 1000;
    let (y, m, d) = civil_from_days((seconds / 86_400) as i64);
    let in_day = seconds % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
        millis % 1000
    )
}

// --- attribution ------------------------------------------------------------------------------------

/// One response of a session log, reduced to what attribution needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) session: String,
    /// Milliseconds since the Unix epoch.
    pub(crate) ts: u64,
    pub(crate) model: String,
    pub(crate) tokens: Tokens,
    /// Whether a subagent made the call.
    pub(crate) subagent: bool,
    /// The reasoning effort the log recorded for the call, when it recorded one.
    pub(crate) effort: Option<String>,
}

/// How far ahead of this machine's clock a log entry may be.
pub(crate) const FUTURE_SLACK_MS: u64 = 60_000;

/// The last mark at or before `ts`, when `ts` is inside the run's window.
pub(crate) fn mark_at(run: &Run, ts: u64) -> Option<&Mark> {
    if run.finished_at.is_some_and(|end| ts > end) {
        return None;
    }
    run.timeline.iter().rev().find(|mark| mark.at <= ts)
}

/// Whether two runs were active at the same time.
pub(crate) fn overlaps(a: &Run, b: &Run) -> bool {
    let window = |run: &Run| {
        (
            run.timeline.first().map_or(u64::MAX, |m| m.at),
            run.finished_at.unwrap_or(u64::MAX),
        )
    };
    let ((a_start, a_end), (b_start, b_end)) = (window(a), window(b));
    a_start <= b_end && b_start <= a_end
}

fn claims(run: &Run, session: &RunSession) -> bool {
    session.mentions.contains(&run.id) || run.sessions.contains(&session.id)
}

/// Where a mark's work is counted: a ticket, the rework, or neither.
fn bucket(mark: &Mark) -> (Option<String>, bool) {
    let rework = mark.stage == Stage::Build && mark.rework.is_some();
    let ticket = if mark.stage == Stage::Build && !rework {
        mark.ticket.clone()
    } else {
        None
    };
    (ticket, rework)
}

/// The usage of `target` from the sessions that were read. `runs` holds every run, so an entry that
/// two runs could both claim is assigned to neither.
pub(crate) fn attribute(
    target: &Run,
    runs: &[Run],
    sessions: &[RunSession],
    now: u64,
) -> (Vec<UsageRow>, Tokens, Vec<String>) {
    type Key = (Stage, String, Option<String>, bool);
    let mut rows: BTreeMap<Key, UsageRow> = BTreeMap::new();
    let mut unattributed = Tokens::default();
    let mut counted: Vec<String> = Vec::new();
    for session in sessions.iter().filter(|session| claims(target, session)) {
        let claimants: Vec<&Run> = runs.iter().filter(|run| claims(run, session)).collect();
        let mut used = false;
        for entry in &session.entries {
            // An entry from the future cannot be the run's work: a clock is wrong or the log is made up.
            if entry.ts > now.saturating_add(FUTURE_SLACK_MS) {
                continue;
            }
            let Some(mark) = mark_at(target, entry.ts) else {
                continue;
            };
            let contested = claimants
                .iter()
                .filter(|run| run.id != target.id && mark_at(run, entry.ts).is_some())
                .count();
            if contested > 0 {
                unattributed.add(&entry.tokens);
                continue;
            }
            let (ticket, rework) = bucket(mark);
            let row = rows
                .entry((mark.stage, entry.model.clone(), ticket.clone(), rework))
                .or_insert_with(|| UsageRow {
                    stage: mark.stage,
                    model: entry.model.clone(),
                    ticket,
                    rework,
                    tokens: Tokens::default(),
                    subagents: Tokens::default(),
                    long_context: Tokens::default(),
                    efforts: BTreeSet::new(),
                });
            if let Some(effort) = &entry.effort
                && (row.efforts.len() < MAX_EFFORTS_PER_ROW || row.efforts.contains(effort))
            {
                row.efforts.insert(effort.clone());
            }
            row.tokens.add(&entry.tokens);
            if entry.subagent {
                row.subagents.add(&entry.tokens);
            }
            let request_input = [
                entry.tokens.input,
                entry.tokens.cache_read,
                entry.tokens.cache_write,
            ]
            .iter()
            .fold(0u64, |sum, n| sum.saturating_add(n.unwrap_or(0)));
            if request_input > LONG_CONTEXT_INPUT_TOKENS {
                row.long_context.add(&entry.tokens);
            }
            used = true;
        }
        if used {
            counted.push(session.id.clone());
        }
    }
    // Stages in order, then the rework, the tickets in the run's order, and work with no ticket last.
    let ticket_order = |row: &UsageRow| match (&row.ticket, row.rework) {
        (Some(reference), _) => target
            .tickets
            .iter()
            .position(|t| t.reference == *reference)
            .unwrap_or(usize::MAX - 2),
        (None, true) => usize::MAX - 1,
        (None, false) => usize::MAX,
    };
    let mut rows: Vec<UsageRow> = rows.into_values().collect();
    rows.sort_by(|a, b| {
        (a.stage, ticket_order(a), &a.model).cmp(&(b.stage, ticket_order(b), &b.model))
    });
    (rows, unattributed, counted)
}

/// The pauses and the idle gaps of `target` in the sessions it claims, inside its window. A gap that a
/// pause explains is not also an idle gap.
pub(crate) fn derive_pauses(
    target: &Run,
    sessions: &[RunSession],
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
    let claimed = sessions.iter().filter(|session| claims(target, session));
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

/// Reads the session logs and works out the usage of `target`. Never fails: whatever cannot be read
/// becomes a note, or a reason when nothing at all is known.
pub(crate) fn collect(
    target: &Run,
    runs: &[Run],
    roots: &Roots,
    now: u64,
    deadline: Instant,
) -> Usage {
    collect_limited(target, runs, roots, now, deadline, ReadLimits::default())
}

#[expect(
    clippy::too_many_lines,
    reason = "does several steps in one function; deepening candidate"
)]
fn collect_limited(
    target: &Run,
    runs: &[Run],
    roots: &Roots,
    now: u64,
    deadline: Instant,
    limits: ReadLimits,
) -> Usage {
    let Some(first) = target.timeline.first() else {
        return Usage::not_tracked(
            "the run has no stage timeline: it started before usage tracking existed",
            now,
        );
    };
    // Only runs whose time overlaps the target's can contest its entries, so only they are looked for;
    // this also keeps the scan from reaching back to the oldest run there is.
    let tracked: Vec<&Run> = runs
        .iter()
        .filter(|run| run.id == target.id || (!run.timeline.is_empty() && overlaps(target, run)))
        .collect();
    let ids: HashSet<&str> = tracked.iter().map(|run| run.id.as_str()).collect();
    let recorded: HashSet<&str> = tracked
        .iter()
        .flat_map(|run| run.sessions.iter().map(String::as_str))
        .collect();
    let from = tracked
        .iter()
        .filter_map(|run| run.timeline.first())
        .map(|mark| mark.at)
        .min()
        .unwrap_or(first.at);
    let read = read_sessions(roots, &ids, &recorded, from, deadline, limits);
    let claimed: Vec<&RunSession> = read.sessions.iter().filter(|s| claims(target, s)).collect();
    if claimed.is_empty() {
        let reason = if read.roots_found == 0 {
            "no Claude Code, Codex or Copilot session logs were found on this machine".to_string()
        } else if read.timed_out {
            "reading the session logs took too long".to_string()
        } else if read.truncated {
            "reading the session logs stopped at its limit on their number and size".to_string()
        } else {
            "no session log names this run and none was recorded for it; its agent may not write logs this tool reads, or its session id was never recorded".to_string()
        };
        return Usage::not_tracked(reason, now);
    }
    let (rows, unattributed, counted) = attribute(target, runs, &read.sessions, now);
    let (pauses, idle) = derive_pauses(target, &read.sessions, now);
    let log_complete = read.roots_found > 0 && !read.timed_out && !read.truncated;
    let mut notes: Vec<String> = Vec::new();
    for note in &read.notes {
        if note
            .session
            .as_ref()
            .is_none_or(|id| claimed.iter().any(|s| s.id == *id))
            && !notes.contains(&note.text)
        {
            notes.push(note.text.clone());
        }
    }
    if read.timed_out {
        notes.push("reading the session logs took too long; usage may be incomplete".into());
    }
    if read.truncated {
        notes.push("reading the session logs stopped at its limit on their number and size; usage may be incomplete".into());
    }
    if target.timeline.len() >= crate::run::MAX_MARKS {
        notes.push(
            "the stage timeline reached its limit, so later work is attributed to its last entry"
                .into(),
        );
    }
    if !unattributed.is_empty() {
        notes.push(format!(
            "{} fresh tokens were used in a session shared with another run at the same time and are not assigned to either",
            unattributed.fresh().map_or_else(|| "some".to_string(), |n| n.to_string())
        ));
    }
    let created = target
        .id
        .parse::<u128>()
        .ok()
        .map(|nanos| (nanos / NANOS_PER_MS) as u64);
    if created.is_some_and(|created| first.at > created + RUN_START_SLACK_MS) {
        notes.push(format!(
            "usage before {} was not tracked: the run's stage timeline starts then",
            format_timestamp(first.at)
        ));
    }
    if rows.is_empty() {
        let reason = if unattributed.is_empty() {
            "the matched session logs hold no usage entries inside the run's stages in a recognised format"
        } else {
            "all usage in the matched sessions was shared with another run working at the same time, so none could be assigned"
        };
        let mut usage = Usage::not_tracked(reason, now);
        usage.notes = notes;
        usage.unattributed = unattributed;
        usage.pauses = pauses;
        usage.idle = idle;
        usage.log_complete = log_complete;
        return usage;
    }
    let mut no_data: Vec<Stage> = Vec::new();
    for mark in &target.timeline {
        if !rows.iter().any(|row| row.stage == mark.stage) && !no_data.contains(&mark.stage) {
            no_data.push(mark.stage);
        }
    }
    let partial = !notes.is_empty() || !no_data.is_empty();
    Usage {
        status: if partial {
            UsageStatus::Partial
        } else {
            UsageStatus::Tracked
        },
        reason: None,
        notes,
        no_data,
        collected_at: now,
        sessions: counted.into_iter().take(MAX_SESSIONS_LISTED).collect(),
        rows,
        unattributed,
        pauses,
        idle,
        log_complete,
    }
}

/// Run ids are creation times in nanoseconds; usage timestamps are milliseconds.
const NANOS_PER_MS: u128 = 1_000_000;

/// Clock and write skew allowed between a run's creation and its first usage entry before a note is added.
const RUN_START_SLACK_MS: u64 = 5_000;

/// The most sessions a usage report lists: enough for any run, and a bound on the size of the dashboard payload.
const MAX_SESSIONS_LISTED: usize = 100;

/// The first settle pass: soon after the run, to show nearly complete numbers quickly.
const FIRST_SETTLE_DELAY: std::time::Duration = std::time::Duration::from_secs(15);

/// The second settle pass: by now agents have surely flushed their logs.
const SECOND_SETTLE_DELAY: std::time::Duration = std::time::Duration::from_secs(45);

// --- collecting -------------------------------------------------------------------------------------

/// How usage is collected: where the logs are, and whether collection is switched on.
#[derive(Clone, Debug)]
pub(crate) struct UsageSettings {
    pub(crate) enabled: bool,
    pub(crate) roots: Roots,
    /// How long one collection may read logs.
    pub(crate) budget: std::time::Duration,
    /// Agents write their logs a little after the fact, so the last moments of work are missing from a
    /// collection made at once. Each delay is the wait before one more pass; each pass recomputes everything.
    pub(crate) settle: Vec<std::time::Duration>,
}

impl UsageSettings {
    /// The delays between passes in the dashboard: soon after, then once the logs have surely been written.
    pub(crate) fn default_settle() -> Vec<std::time::Duration> {
        vec![FIRST_SETTLE_DELAY, SECOND_SETTLE_DELAY]
    }
}

/// The reason shown when the user switched collection off.
pub(crate) const SWITCHED_OFF: &str =
    "usage collection is switched off (JIDOKA_USAGE=off or jidoka dashboard --no-usage)";

/// Whether `JIDOKA_USAGE` asks for collection to be off.
pub(crate) fn switched_off_by(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "false" | "0" | "no"
        )
    })
}

/// Whether two readings hold the same counts. When they were taken and whether the log was read completely
/// (not stored) are not counts.
fn same_counts(a: &Usage, b: &Usage) -> bool {
    Usage {
        collected_at: 0,
        log_complete: false,
        ..a.clone()
    } == Usage {
        collected_at: 0,
        log_complete: false,
        ..b.clone()
    }
}

/// Counts only ever grow while the logs stay: when a recomputation finds less than is stored, a log was
/// deleted or could not be read, so the stored counts are kept and marked as possibly incomplete.
fn keep_more(old: Option<&Usage>, new: Usage) -> Usage {
    let Some(old) = old else { return new };
    let lost = old.status != UsageStatus::NotTracked
        && match (old.fresh(), new.fresh()) {
            (Some(before), Some(now)) => now < before,
            (Some(_), None) => true,
            _ => false,
        };
    if !lost {
        // Not tracked now, but counted before: a reason must not wipe counts, handled by `lost`.
        return new;
    }
    let mut kept = old.clone();
    kept.status = UsageStatus::Partial;
    kept.collected_at = new.collected_at;
    kept.log_complete = false;
    if !new.pauses.is_empty() {
        kept.pauses = new.pauses;
    }
    kept.idle = new.idle;
    let note = "the counts kept from an earlier reading are higher than the latest reading found: a session log is no longer available, or its work is now shared with another run";
    if !kept.notes.iter().any(|n| n == note) {
        kept.notes.push(note.to_string());
    }
    kept
}

/// Serializes usage collection for all handles to one run store.
#[derive(Default)]
pub(crate) struct Collection {
    lock: std::sync::Mutex<()>,
}

impl Collection {
    fn collect(
        &self,
        store: &crate::run::RunStore,
        id: &str,
        settings: &UsageSettings,
    ) -> std::io::Result<()> {
        self.collect_with(store, id, settings, || {}, || Ok(()))
    }

    fn collect_with(
        &self,
        store: &crate::run::RunStore,
        id: &str,
        settings: &UsageSettings,
        waiting: impl FnOnce(),
        acquired: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let _guard = match self.lock.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(error)) => {
                return Err(crate::run::lock_error(error));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                waiting();
                self.lock.lock().map_err(crate::run::lock_error)?
            }
        };
        acquired()?;
        collect_locked(store, id, settings)
    }

    #[cfg(test)]
    #[expect(
        clippy::too_many_arguments,
        reason = "takes one parameter per input instead of a parameter struct; deepening candidate"
    )]
    pub(crate) fn collect_paused(
        &self,
        store: &crate::run::RunStore,
        id: &str,
        settings: &UsageSettings,
        waiting: Option<std::sync::mpsc::Sender<()>>,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) -> std::io::Result<()> {
        self.collect_with(
            store,
            id,
            settings,
            || {
                if let Some(waiting) = waiting {
                    let _ = waiting.send(());
                }
            },
            || {
                entered.send(()).map_err(std::io::Error::other)?;
                release.recv().map_err(std::io::Error::other)
            },
        )
    }
}

/// Recomputes the usage of one run from the logs and stores it. Collections never overlap. A failure
/// is returned, never panics, and leaves the stored usage as it was; the caller only reports it.
pub(crate) fn collect_and_store(
    store: &crate::run::RunStore,
    id: &str,
    settings: &UsageSettings,
) -> std::io::Result<()> {
    store.usage_collection().collect(store, id, settings)
}

fn collect_locked(
    store: &crate::run::RunStore,
    id: &str,
    settings: &UsageSettings,
) -> std::io::Result<()> {
    let runs = store.list()?;
    let Some(target) = runs.iter().find(|run| run.id == id) else {
        return Ok(());
    };
    let now = crate::run::updated_now()?;
    let fresh = if settings.enabled {
        collect(
            target,
            &runs,
            &settings.roots,
            now,
            Instant::now() + settings.budget,
        )
    } else {
        Usage::not_tracked(SWITCHED_OFF, now)
    };
    let usage = if settings.enabled {
        keep_more(target.usage.as_ref(), fresh)
    } else {
        fresh
    };
    let latest = usage.pauses.last().cloned();
    let authoritative = usage.log_complete;
    if target.model_routing.is_some()
        || !target
            .usage
            .as_ref()
            .is_some_and(|old| same_counts(old, &usage))
    {
        store.set_usage(id, usage)?;
    }
    // "No pause in the log" only counts when the whole log was read: a run is never resumed on a reading
    // that is off, partial or empty.
    if latest.is_none() && !authoritative {
        return Ok(());
    }
    store.apply_pause(id, latest.as_ref())
}

/// How long a run may go without an update before it is no longer watched for pauses.
const WATCH_WINDOW_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// One watching pass: collects the runs that are working or paused on their own, so a pause is seen
/// and cleared without an agent update. Done runs and runs long without an update are left alone.
pub(crate) fn collect_active(
    store: &crate::run::RunStore,
    settings: &UsageSettings,
    now: u64,
) -> std::io::Result<()> {
    if !settings.enabled {
        return Ok(());
    }
    let mut first_error = None;
    for run in store.list()? {
        let automatic = crate::run::is_automatic_reason(run.status, run.blocking_reason.as_deref());
        let watched = run.status == crate::run::RunStatus::Running || automatic;
        let recent = run
            .updated_at
            .is_some_and(|at| at.saturating_add(WATCH_WINDOW_MS) >= now);
        if watched && recent {
            // One failing run must not stop the others from being checked.
            if let Err(error) = collect_and_store(store, &run.id, settings) {
                eprintln!("Run {} was not checked for pauses: {error}", run.id);
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
