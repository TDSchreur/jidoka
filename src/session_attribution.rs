//! Which model responses of the agents' session logs belong to a Run, for Token usage and the request
//! analysis alike. One scan reads the logs from the Run's Agent homes within the read limits, finds the
//! sessions the Run claims (the Session claim in the glossary), and places each response under the
//! Run's Timeline. A response that another Run working at the same time could also claim is ambiguous and
//! belongs to neither. Totals and categories stay with the reports.

use crate::{
    agent_logs::{LogSession, ReadLimits, ReadResult, Response, Roots, RunWindow, read_sessions},
    run::Run,
    usage::{FUTURE_SLACK_MS, Mark, mark_at},
};
use std::time::Instant;

/// The reason a Run that started before usage tracking existed cannot be scanned.
pub(crate) const NO_TIMELINE: &str =
    "the run has no stage timeline: it started before usage tracking existed";

/// Start of the note for a run whose recorded sessions no scanned log claims.
pub(crate) const RECORDED_NOT_FOUND: &str =
    "the run has recorded sessions, but none of their logs were found";

/// The session logs one scan read for a Run, with the runs that could contest its sessions.
pub(crate) struct SessionScan<'a> {
    pub(crate) target: &'a Run,
    /// The first mark of the Run's Timeline, where its time starts.
    pub(crate) first: &'a Mark,
    /// The target and the runs whose time overlaps it, which are the only ones that can contest it.
    pub(crate) window: RunWindow<'a>,
    /// The log folders scanned: the Agent homes the Run recorded, otherwise this environment's.
    pub(crate) roots: Roots,
    /// The sessions read, their notes, and whether the reading was cut short.
    pub(crate) read: ReadResult,
}

/// Why a scan found no session the Run claims.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Missing {
    /// The Run recorded sessions, but no scanned log is one of them; the note names them and the folders.
    RecordedNotFound(String),
    /// None of the agents' log folders exist.
    NoLogs,
    /// Logs were read, but none executed a Jidoka command naming the Run.
    Unclaimed,
}

/// How long before a Run's first Timeline mark some responses still count for it.
#[derive(Clone, Copy)]
pub(crate) struct LeadIn {
    pub(crate) ms: u64,
    /// Which responses the lead-in applies to.
    pub(crate) applies: fn(&Response) -> bool,
}

/// A response the Run can claim, with the Timeline mark it falls under.
pub(crate) struct Attributed<'s> {
    pub(crate) response: &'s Response,
    /// The last mark at or before the response; the first mark for a response in the lead-in.
    pub(crate) mark: &'s Mark,
}

/// The responses of the Run's claimed sessions, split into the Run's own and the ambiguous ones.
pub(crate) struct Attribution<'s> {
    /// In session id order, then in the order of each session's log.
    pub(crate) responses: Vec<Attributed<'s>>,
    /// Responses inside the Run's time that another overlapping Run claiming the session could also claim.
    pub(crate) ambiguous: Vec<&'s Response>,
    /// The sessions with at least one attributed response, in id order.
    pub(crate) sessions: Vec<&'s LogSession>,
}

/// Reads the session logs that may hold `target`'s work, within `deadline` and `limits`. `runs` holds
/// every run, so the runs that overlap `target` are read too. The Run's recorded Agent homes replace
/// `roots` for their agents.
///
/// # Errors
///
/// Returns [`NO_TIMELINE`] when `target` has no Timeline, so no response can be placed in it.
pub(crate) fn scan<'a>(
    target: &'a Run,
    runs: &'a [Run],
    roots: &Roots,
    deadline: Instant,
    limits: ReadLimits,
) -> Result<SessionScan<'a>, &'static str> {
    let Some(first) = target.timeline.first() else {
        return Err(NO_TIMELINE);
    };
    // Only runs whose time overlaps the target's can contest its responses, so only they are looked for;
    // this also keeps the scan from reaching back to the oldest run there is.
    let window = RunWindow::new(target, runs, first.at);
    let roots = roots.for_run(target);
    let read = read_sessions(&roots, &window, deadline, limits);
    Ok(SessionScan {
        target,
        first,
        window,
        roots,
        read,
    })
}

impl<'a> SessionScan<'a> {
    /// The sessions the Run claims, in id order.
    pub(crate) fn claimed(&self) -> Vec<&LogSession> {
        self.read
            .sessions
            .iter()
            .filter(|session| session.belongs_to(self.target))
            .collect()
    }

    /// Why no session is claimed, or `None` when one is. Whether the reading was cut short is in `read`.
    pub(crate) fn missing(&self) -> Option<Missing> {
        if !self.claimed().is_empty() {
            return None;
        }
        Some(if !self.target.sessions.is_empty() {
            Missing::RecordedNotFound(self.recorded_not_found_note())
        } else if self.read.roots_found == 0 {
            Missing::NoLogs
        } else {
            Missing::Unclaimed
        })
    }

    /// The note for a run whose recorded sessions no scanned log claims: names the sessions and the folders.
    fn recorded_not_found_note(&self) -> String {
        let roots = &self.roots;
        // CLAUDE_CONFIG_DIR points to the directory that contains projects.
        let claude = roots.claude.parent().unwrap_or(&roots.claude);
        format!(
            "{RECORDED_NOT_FOUND}: {} (scanned folders: {}, {}, {}); for Claude Code, set CLAUDE_CONFIG_DIR to the Claude folder that holds the sessions",
            self.target.sessions.join(", "),
            claude.display(),
            roots.codex.display(),
            roots.copilot.display()
        )
    }

    /// The responses of the claimed sessions that fall in the Run's time, each placed under its Timeline
    /// mark. A response later than `now` (with slack) is left out: a clock is wrong or the log is made up.
    /// With `lead_in`, a response it applies to also counts shortly before the first mark, for the Run and
    /// for its rivals alike.
    pub(crate) fn attribute(&self, now: u64, lead_in: Option<LeadIn>) -> Attribution<'_> {
        let mut attribution = Attribution {
            responses: Vec::new(),
            ambiguous: Vec::new(),
            sessions: Vec::new(),
        };
        for session in self.claimed() {
            let rivals: Vec<&Run> = self
                .window
                .tracked
                .iter()
                .copied()
                .filter(|run| run.id != self.target.id && session.belongs_to(run))
                .collect();
            let mut used = false;
            for response in &session.responses {
                if response.ts > now.saturating_add(FUTURE_SLACK_MS) {
                    continue;
                }
                let Some(mark) = placed(self.target, response, lead_in) else {
                    continue;
                };
                if rivals
                    .iter()
                    .any(|run| placed(run, response, lead_in).is_some())
                {
                    attribution.ambiguous.push(response);
                    continue;
                }
                attribution.responses.push(Attributed { response, mark });
                used = true;
            }
            if used {
                attribution.sessions.push(session);
            }
        }
        attribution
    }
}

/// The mark of `run` that `response` falls under: the last one at or before it inside the Run's time, or
/// the first one when the response is in the lead-in.
fn placed<'r>(run: &'r Run, response: &Response, lead_in: Option<LeadIn>) -> Option<&'r Mark> {
    if let Some(mark) = mark_at(run, response.ts) {
        return Some(mark);
    }
    let lead_in = lead_in?;
    let first = run.timeline.first()?;
    (response.ts < first.at && first.at - response.ts <= lead_in.ms && (lead_in.applies)(response))
        .then_some(first)
}

#[cfg(test)]
#[path = "session_attribution_tests.rs"]
mod tests;
