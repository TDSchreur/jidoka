//! `jidoka run requests`: the model requests of a run per category, with their tokens.

use crate::{
    agent_logs::{LogSession, ReadLimits, Roots, RunWindow, read_sessions},
    inspect::shown,
    request_logs::{Category, Request, request_of},
    run::Run,
    stats::{number, table_with},
    usage::{FUTURE_SLACK_MS, Tokens, mark_at},
};
use serde::Serialize;
use std::{collections::BTreeMap, time::Instant};

/// Requests and tokens of one category.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Counts {
    pub(crate) requests: u64,
    /// The bytes of text the tool calls of these requests returned, and the most one request got.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) largest_tool_output_bytes: Option<u64>,
    #[serde(flatten)]
    pub(crate) tokens: Tokens,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CategoryRow {
    pub(crate) category: Category,
    /// Whether these are the requests of subagents, shown apart from the coordinator's.
    pub(crate) subagent: bool,
    #[serde(flatten)]
    pub(crate) counts: Counts,
}

/// The requests of one run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RunRequests {
    pub(crate) run: String,
    pub(crate) title: String,
    pub(crate) categories: Vec<CategoryRow>,
    pub(crate) total: Counts,
    /// Requests that another run working at the same time could also claim; in neither run.
    pub(crate) ambiguous: Counts,
    /// Of the total, the requests of subagents.
    pub(crate) subagents: Counts,
    /// Sessions of the run with no subagent logs: their figures cover the coordinator only.
    pub(crate) coordinator_only_sessions: Vec<String>,
    /// Requests whose commands also updated the run, next to other work. They are counted in their own
    /// category; this is a count beside the categories, not one of them.
    pub(crate) chained_reporting: u64,
    /// Requests whose commands fit more than one category.
    pub(crate) several_categories: u64,
    pub(crate) sessions: Vec<String>,
    pub(crate) notes: Vec<String>,
}

impl Counts {
    /// Adds `other` into these counts. A tool output size known on one side only stays known.
    pub(crate) fn merge(&mut self, other: &Counts) {
        self.requests += other.requests;
        self.tokens.add(&other.tokens);
        if let Some(bytes) = other.tool_output_bytes {
            self.tool_output_bytes =
                Some(self.tool_output_bytes.unwrap_or(0).saturating_add(bytes));
        }
        if let Some(bytes) = other.largest_tool_output_bytes {
            self.largest_tool_output_bytes =
                Some(self.largest_tool_output_bytes.unwrap_or(0).max(bytes));
        }
    }
}

fn add(counts: &mut Counts, request: &Request) {
    counts.requests += 1;
    counts.tokens.add(&request.tokens);
    if let Some(bytes) = request.tool_output_bytes {
        counts.tool_output_bytes =
            Some(counts.tool_output_bytes.unwrap_or(0).saturating_add(bytes));
        counts.largest_tool_output_bytes =
            Some(counts.largest_tool_output_bytes.unwrap_or(0).max(bytes));
    }
}

/// How long before a run's first mark the request that starts it may be. The model writes that request
/// before `jidoka run start` executes and creates the run, so it always falls just before the first mark.
const START_GRACE_MS: u64 = 120_000;

/// Whether `request` is the run's work: it falls inside the run's time, or it is workspace work just
/// before the run's first mark.
fn in_run(run: &Run, request: &Request) -> bool {
    mark_at(run, request.ts).is_some()
        || (request.category == Category::Workspace
            && run.timeline.first().is_some_and(|first| {
                request.ts < first.at && first.at - request.ts <= START_GRACE_MS
            }))
}

/// Start of the note for a run whose recorded sessions no scanned log claims.
pub(crate) const RECORDED_NOT_FOUND: &str =
    "the run has recorded sessions, but none of their logs were found";

/// The note for a run whose recorded sessions no scanned log claims: names the sessions and the scanned folders.
pub(crate) fn recorded_not_found_note(target: &Run, roots: &Roots) -> String {
    // CLAUDE_CONFIG_DIR points to the directory that contains projects.
    let claude = roots.claude.parent().unwrap_or(&roots.claude);
    format!(
        "{RECORDED_NOT_FOUND}: {} (scanned folders: {}, {}, {}); for Claude Code, set CLAUDE_CONFIG_DIR to the Claude folder that holds the sessions",
        target.sessions.join(", "),
        claude.display(),
        roots.codex.display(),
        roots.copilot.display()
    )
}

/// The requests of `target` from the Claude Code logs. `runs` holds every run, so a request that two runs
/// working at the same time could both claim is in neither. Never fails: whatever cannot be read becomes a
/// note.
#[expect(
    clippy::too_many_lines,
    reason = "reads logs per agent and builds categories, subagent and ambiguity counts together"
)]
pub(crate) fn analyze(
    target: &Run,
    runs: &[Run],
    roots: &Roots,
    now: u64,
    deadline: Instant,
) -> RunRequests {
    let mut result = RunRequests {
        run: target.id.clone(),
        title: target.title.clone(),
        categories: Vec::new(),
        total: Counts::default(),
        ambiguous: Counts::default(),
        subagents: Counts::default(),
        coordinator_only_sessions: Vec::new(),
        chained_reporting: 0,
        several_categories: 0,
        sessions: Vec::new(),
        notes: Vec::new(),
    };
    let Some(first) = target.timeline.first() else {
        result
            .notes
            .push("the run has no stage timeline: it started before usage tracking existed".into());
        return result;
    };
    let window = RunWindow::new(target, runs, first.at);
    let tracked = &window.tracked;
    // The run's own agent homes, when it recorded them, so the notes below name the folders scanned.
    let roots = &roots.for_run(target);
    let read = read_sessions(roots, &window, deadline, ReadLimits::default());
    for note in &read.notes {
        if !result.notes.contains(&note.text) {
            result.notes.push(note.text.clone());
        }
    }
    let (timed_out, truncated) = (read.timed_out, read.truncated);
    if timed_out {
        result
            .notes
            .push("reading the session logs took too long; the counts may be incomplete".into());
    }
    if truncated {
        result.notes.push(
            "reading the session logs stopped at its limit on their number and size; the counts may be incomplete"
                .into(),
        );
    }
    let claimed: Vec<&LogSession> = read
        .sessions
        .iter()
        .filter(|session| session.belongs_to(target))
        .collect();
    if claimed.is_empty() {
        result.notes.push(if !target.sessions.is_empty() {
            recorded_not_found_note(target, roots)
        } else if read.roots_found == 0 {
            "no Claude Code, Codex or Copilot session logs were found on this machine".to_string()
        } else {
            "no session log executed a jidoka run update for this run and none was recorded for it"
                .to_string()
        });
        return result;
    }
    let mut by_category: BTreeMap<(Category, bool), Counts> = BTreeMap::new();
    for session in claimed {
        let rivals: Vec<&Run> = tracked
            .iter()
            .copied()
            .filter(|run| run.id != target.id && session.belongs_to(run))
            .collect();
        let mut used = false;
        for response in &session.responses {
            let request = &request_of(response);
            // A request from the future cannot be the run's work: a clock is wrong or the log is made up.
            if request.ts > now.saturating_add(FUTURE_SLACK_MS) || !in_run(target, request) {
                continue;
            }
            if rivals.iter().any(|run| in_run(run, request)) {
                add(&mut result.ambiguous, request);
                continue;
            }
            add(
                by_category
                    .entry((request.category, request.subagent))
                    .or_default(),
                request,
            );
            add(&mut result.total, request);
            if request.subagent {
                add(&mut result.subagents, request);
            }
            result.chained_reporting += u64::from(request.chained_reporting);
            result.several_categories += u64::from(request.several_categories);
            used = true;
        }
        if used {
            result.sessions.push(session.id.clone());
            if !session.has_subagent_logs {
                result.coordinator_only_sessions.push(session.id.clone());
            }
        }
    }
    result.categories = by_category
        .into_iter()
        .map(|((category, subagent), counts)| CategoryRow {
            category,
            subagent,
            counts,
        })
        .collect();
    if result.subagents.requests > 0 {
        result.notes.push(format!(
            "Subagent requests are included: {} of the {} requests",
            result.subagents.requests, result.total.requests
        ));
    }
    if !result.coordinator_only_sessions.is_empty() {
        result.notes.push(format!(
            "{} of {} sessions have no subagent logs, so their figures are coordinator-only",
            result.coordinator_only_sessions.len(),
            result.sessions.len()
        ));
    }
    if result
        .categories
        .iter()
        .any(|row| row.category == Category::NotCategorized)
    {
        result.notes.push(
            "Some Copilot requests have no matching assistant message in the log, so they are not categorized and have no tool output size; the tool output totals leave them out".into(),
        );
    }
    if result.ambiguous.requests > 0 {
        result.notes.push(format!(
            "{} requests were made in a session shared with another run at the same time and are in neither run",
            result.ambiguous.requests
        ));
    }
    if result.total.requests == 0 {
        result
            .notes
            .push("the matched session logs hold no requests inside the run's time".into());
    }
    result
}

fn share(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "-".into();
    }
    format!("{:.1}%", part as f64 * 100.0 / whole as f64)
}

pub(crate) fn cells(label: &str, counts: &Counts, total: u64) -> Vec<String> {
    vec![
        label.to_string(),
        number(Some(counts.requests)),
        share(counts.requests, total),
        number(counts.tokens.input),
        number(counts.tokens.cache_read),
        number(counts.tokens.cache_write),
        number(counts.tokens.output),
        number(counts.tool_output_bytes),
        number(counts.largest_tool_output_bytes),
    ]
}

fn plural(count: u64) -> &'static str {
    if count == 1 { "request" } else { "requests" }
}

const EXPLANATION: &str = "A model request is one response of the model, counted once. Each has one category from its Bash commands. Tokens are separate columns; n/a means the log did not report it, and it is never zero. Tool output is the bytes of text the tool calls returned.";

pub(crate) fn render_run(result: &RunRequests) -> String {
    let mut out = format!(
        "Model requests of run {}: {}\n\n{EXPLANATION}\n\n",
        shown(&result.run),
        shown(&result.title)
    );
    let total = result.total.requests;
    let mut rows: Vec<Vec<String>> = result
        .categories
        .iter()
        .map(|row| {
            let label = if row.subagent {
                format!("{} (subagents)", row.category.label())
            } else {
                row.category.label().to_string()
            };
            cells(&label, &row.counts, total)
        })
        .collect();
    if total > 0 {
        rows.push(cells("total", &result.total, total));
        if result.subagents.requests > 0 {
            rows.push(cells("of which subagents", &result.subagents, total));
        }
        out.push_str(&table_with(
            &rows,
            &[
                "Category",
                "Requests",
                "Share",
                "Input",
                "Cache read",
                "Cache write",
                "Output",
                "Tool output",
                "Largest",
            ],
            1,
        ));
    } else {
        out.push_str("  No requests were counted.\n");
    }
    if total > 0 {
        out.push_str(&format!(
            "\nChained reporting: {} {} (counted in their own categories)\nRequests that fit more than one category: {} {} (each counted in the first)\n",
            result.chained_reporting,
            plural(result.chained_reporting),
            result.several_categories,
            plural(result.several_categories),
        ));
    }
    if result.ambiguous.requests > 0 {
        out.push_str(&format!(
            "\nNot assigned to either run: {} requests\n",
            number(Some(result.ambiguous.requests))
        ));
    }
    if !result.notes.is_empty() {
        out.push_str("\nWhat these figures do not cover\n");
        for note in &result.notes {
            out.push_str(&format!("  - {}\n", shown(note)));
        }
    }
    out
}

#[cfg(test)]
#[path = "requests_tests.rs"]
mod tests;
