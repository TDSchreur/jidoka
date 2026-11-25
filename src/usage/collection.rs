//! Collecting a run's Token usage: reading the session logs, attributing each entry to the run's stage, model
//! and ticket by its Timeline, and storing the result with the run.
//!
//! A session belongs to a run only when its id was recorded on the run or its log names the run id; never by
//! folder or time alone. An entry that two runs could both claim is not assigned to either. Unknown usage is
//! never shown as zero: a field a log does not report is absent.

use crate::{
    run::{Run, Stage},
    timestamps,
    usage::{
        agent_logs::{LogSession, ReadLimits, Roots},
        pauses::{Pause, PauseKind, derive_pauses},
        session_attribution::{self, Attributed, Attribution, Mark, Missing, mark_at},
        token_usage::{
            LONG_CONTEXT_INPUT_TOKENS, MAX_EFFORTS_PER_ROW, Tokens, Usage, UsageRow, UsageStatus,
            UsedSkill,
        },
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

/// How long before a run's first stage a skill still counts for it: the workflow skill is loaded just
/// before `jidoka run begin` creates the run.
const SKILL_LEAD_MS: u64 = 5 * 60 * 1000;

/// Whether a skill used at `at` could belong to `run`: it is inside the run's stages, or in the lead
/// before its first one.
fn skill_window(run: &Run, at: u64) -> bool {
    mark_at(run, at).is_some()
        || run
            .timeline
            .first()
            .is_some_and(|first| at < first.at && at + SKILL_LEAD_MS >= first.at)
}

/// The skills the sessions that `target` claims used, in time order, each with the stage it fell in.
/// A skill used while another run claiming the same session was active is left out: it cannot be
/// told apart. `tracked` holds the runs that overlap `target`.
fn used_skills(target: &Run, tracked: &[&Run], sessions: &[&LogSession]) -> Vec<UsedSkill> {
    let Some(first) = target.timeline.first() else {
        return Vec::new();
    };
    let mut used = Vec::new();
    for session in sessions {
        for (at, name) in &session.skills {
            let stage = if *at < first.at {
                (*at + SKILL_LEAD_MS >= first.at).then_some(first.stage)
            } else {
                mark_at(target, *at).map(|mark| mark.stage)
            };
            let shared = tracked.iter().any(|other| {
                other.id != target.id && session.belongs_to(other) && skill_window(other, *at)
            });
            if let (Some(stage), false) = (stage, shared) {
                used.push(UsedSkill {
                    stage,
                    name: name.clone(),
                    at: *at,
                });
            }
        }
    }
    used.sort_by_key(|skill| skill.at);
    used
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

/// The token rows of the Run from its attributed responses, and the tokens of the ambiguous ones, which
/// are assigned to neither run.
fn token_rows(target: &Run, attribution: &Attribution) -> (Vec<UsageRow>, Tokens) {
    type Key = (Stage, String, Option<String>, bool);
    let mut rows: BTreeMap<Key, UsageRow> = BTreeMap::new();
    let mut unattributed = Tokens::default();
    for response in &attribution.ambiguous {
        unattributed.add(&response.tokens);
    }
    for Attributed {
        response: entry,
        mark,
    } in &attribution.responses
    {
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
    (rows, unattributed)
}

/// Reads the session logs and works out the usage of `target`. Never fails: whatever cannot be read
/// becomes a note, or a reason when nothing at all is known.
#[expect(
    clippy::too_many_lines,
    reason = "derives rows, skills, pauses, attention and notes from one scan in one function"
)]
pub(crate) fn collect(
    target: &Run,
    runs: &[Run],
    roots: &Roots,
    now: u64,
    deadline: Instant,
) -> Usage {
    let scan = match session_attribution::scan(target, runs, roots, deadline, ReadLimits::default())
    {
        Ok(scan) => scan,
        Err(reason) => return Usage::not_tracked(reason, now),
    };
    let first = scan.first;
    let read = &scan.read;
    let tracked = &scan.window.tracked;
    if let Some(missing) = scan.missing() {
        let reason = match missing {
            Missing::NoLogs => session_attribution::NO_LOGS.to_string(),
            _ if read.timed_out => "reading the session logs took too long".to_string(),
            _ if read.truncated => {
                "reading the session logs stopped at its limit on their number and size".to_string()
            }
            Missing::RecordedNotFound(note) => note,
            Missing::Unclaimed => "no session log names this run and none was recorded for it; its agent may not write logs this tool reads, or its session id was never recorded".to_string(),
        };
        // A log recorded for the run that could not be read is why nothing was found: say so.
        let mut usage = Usage::not_tracked(reason, now);
        for note in &read.notes {
            let recorded = note
                .session
                .as_ref()
                .is_some_and(|id| target.sessions.contains(id));
            if recorded && !usage.notes.contains(&note.text) {
                usage.notes.push(note.text.clone());
            }
        }
        return usage;
    }
    let claimed = scan.claimed();
    let skills = used_skills(target, tracked, &claimed);
    let attribution = scan.attribute(now, None);
    let (rows, unattributed) = token_rows(target, &attribution);
    let counted: Vec<String> = attribution
        .sessions
        .iter()
        .map(|session| session.id.clone())
        .collect();
    let (pauses, idle) = derive_pauses(target, &read.sessions, now);
    // Each session is evaluated on its own signals: another session cannot answer its request.
    // A request left unanswered by a session that a later session has overtaken can no longer be
    // answered, so it must not keep blocking the run.
    let newest_signal_at = |session: &LogSession| session.attention.iter().map(|(at, _)| *at).max();
    let attention = claimed
        .iter()
        .filter_map(|session| {
            let mut latest = session.attention.clone();
            latest.sort_by_key(|(at, _)| *at);
            match latest.last() {
                Some((
                    at,
                    signal @ (crate::usage::attention::Signal::Requested
                    | crate::usage::attention::Signal::Finished),
                )) if *at >= first.at
                    && *at <= now
                    && (*signal != crate::usage::attention::Signal::Requested
                        || !claimed.iter().any(|other| {
                            other.id != session.id
                                && newest_signal_at(other).is_some_and(|n| n > *at)
                        }))
                    && (*signal == crate::usage::attention::Signal::Requested
                        || target
                            .agent_updated_at
                            .or(target.updated_at)
                            .is_none_or(|updated| *at >= updated))
                    && tracked
                        .iter()
                        .filter(|run| {
                            session.belongs_to(run)
                                && run.timeline.first().is_some_and(|mark| mark.at <= *at)
                                && run.finished_at.is_none_or(|end| *at <= end)
                        })
                        .count()
                        == 1 =>
                {
                    Some((
                        Pause {
                            kind: PauseKind::AgentStopped,
                            at: *at,
                            resumed_at: None,
                        },
                        *signal == crate::usage::attention::Signal::Requested,
                    ))
                }
                _ => None,
            }
        })
        .max_by_key(|(pause, requested)| (*requested, pause.at));
    let (attention, attention_requested) = match attention {
        Some((pause, requested)) => (Some(pause), requested),
        None => (None, false),
    };
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
            timestamps::format(first.at)
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
        usage.attention = attention;
        usage.attention_requested = attention_requested;
        usage.skills = skills;
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
        attention,
        attention_requested,
        skills,
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
        attention: None,
        attention_requested: false,
        ..a.clone()
    } == Usage {
        collected_at: 0,
        log_complete: false,
        attention: None,
        attention_requested: false,
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
        reason = "passes each channel and setting separately; a hook struct would collapse these"
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
    // Attention still needs the session logs when token usage is switched off.
    let fresh = collect(
        target,
        &runs,
        &settings.roots,
        now,
        Instant::now() + settings.budget,
    );
    let authoritative = fresh.log_complete;
    let attention_requested = fresh.attention_requested;
    let skills = fresh.skills.clone();
    let attention = fresh.attention.clone();
    let latest = attention.clone().or_else(|| fresh.pauses.last().cloned());
    let usage = if settings.enabled {
        keep_more(target.usage.as_ref(), fresh)
    } else {
        target
            .usage
            .clone()
            .unwrap_or_else(|| Usage::not_tracked(SWITCHED_OFF, now))
    };
    if target.model_routing.is_some()
        || !target
            .usage
            .as_ref()
            .is_some_and(|old| same_counts(old, &usage))
    {
        store.set_usage(id, usage)?;
    }
    // Skills are recorded even when usage is off or unchanged: they come from the same logs.
    store.observe_skills(id, &skills)?;
    // "No pause in the log" only counts when the whole log was read: a run is never resumed on a reading
    // that is off, partial or empty.
    if !authoritative
        && (latest.is_none()
            || (attention.is_none()
                && target.blocking_reason.as_deref() == Some(PauseKind::AgentStopped.reason())))
    {
        return Ok(());
    }
    store.apply_pause(id, latest.as_ref(), attention_requested)
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
    let mut first_error = None;
    for run in store.list()? {
        let automatic = run.automatically_blocked();
        let watched = run.status == crate::run::RunStatus::Running || automatic;
        if watched && updated_recently(&run, now) {
            // One failing run must not stop the others from being checked.
            if let Err(error) = collect_and_store(store, &run.id, settings) {
                eprintln!("Run {} was not checked for pauses: {error}", run.id);
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Whether `run` was updated within the watch window before `now`.
fn updated_recently(run: &Run, now: u64) -> bool {
    run.updated_at
        .is_some_and(|at| at.saturating_add(WATCH_WINDOW_MS) >= now)
}

/// Collects once each done run updated within the watch window that recorded sessions but whose usage
/// is not tracked, so a dashboard started where the run's logs can be read recovers what an earlier one
/// missed. Meant for dashboard startup only: done runs are never watched.
pub(crate) fn collect_missed(
    store: &crate::run::RunStore,
    settings: &UsageSettings,
    now: u64,
) -> std::io::Result<()> {
    if !settings.enabled {
        return Ok(());
    }
    let mut first_error = None;
    for run in store.list()? {
        let missed = run.status == crate::run::RunStatus::Done
            && !run.sessions.is_empty()
            && run
                .usage
                .as_ref()
                .is_some_and(|usage| usage.status == UsageStatus::NotTracked)
            && updated_recently(&run, now);
        if missed && let Err(error) = collect_and_store(store, &run.id, settings) {
            // One failing run must not stop the others from being collected.
            eprintln!("Usage of run {} was not collected: {error}", run.id);
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
#[path = "collection_tests.rs"]
mod tests;
