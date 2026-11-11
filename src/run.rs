use crate::pr_state::PrStatus;
use crate::routing::{ModelChoice, ModelRouting};
use crate::usage::{Collection, Mark, Pause, PauseKind, Usage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const UPDATE_HELP: &str = "Run update options:\n  --activity <text> | --clear-activity\n  --current-ticket <http(s)-url> | --clear-current-ticket\n  --blocking-reason <text> | --clear-blocking-reason\n  --rework <1-99> | --clear-rework\n  --mode <jidoka-full|jidoka-balanced|jidoka-fast> | --clear-mode\n  --worktree <absolute-path> | --clear-worktree\n  --branch <name> | --clear-branch\n  --base-commit <sha> | --clear-base-commit\n  --spec <http(s)-url|path> | --clear-spec\n  --pr <http(s)-url> | --clear-pr\n  --ticket \"<reference> <title>\" (repeat; the flags are the whole ordered list) | --clear-tickets\n  --ticket-status <reference>=<pending|in-progress|done> (repeat)\n  --ticket-report <reference> <skill|activity> <text> (repeat)\n  --run-report <skill|activity> <text> (repeat)\n  --summary <define|build|check|finish>=<text> | --clear-summary <stage> (repeat; `finish done` needs a Finish summary)\n  --session <agent-session-id> (repeat; attributes that session's token usage to this run)\n  --model-routing <define|build|check|finish>=<model>[:<effort>] (repeat; the flags are all of the run's overrides) | --clear-model-routing\nOmitted fields are preserved; clear flags remove them.";
/// Highest Rework count a run can report.
pub(crate) const MAX_REWORK: u32 = 99;
/// Tickets one update may list; a spec split into more pieces than this is too big to track on one board.
const MAX_TICKETS: usize = 100;
/// Sessions one update may attribute to a run; a run rarely spans more than a handful of agent sessions.
const MAX_SESSIONS: usize = 20;
/// Sessions kept per run; the oldest are dropped first.
const KEEP_SESSIONS: usize = 100;
/// Marks kept on a run's timeline, so a long run cannot grow its record without bound.
pub(crate) const MAX_MARKS: usize = 1000;
/// Blocked and resumed events kept per run, so a run that blocks over and over cannot grow its record without bound.
pub(crate) const MAX_EVENTS: usize = 1000;
/// Longest title, ticket title, ticket report or run report, in bytes: enough for a sentence or two that still fits
/// a dashboard card.
const MAX_TEXT_BYTES: usize = 500;
/// Longest path, URL or reference, in bytes; the common 4096-byte `PATH_MAX`, which no real path exceeds.
const MAX_PATH_BYTES: usize = 4096;
/// Longest activity, blocking reason or stage summary, in bytes: a short paragraph.
const MAX_NOTE_BYTES: usize = 2000;
/// Longest git branch name, in bytes; git itself limits a ref name component to 255 bytes on common file systems.
const MAX_BRANCH_BYTES: usize = 255;
/// Shortest and longest agent session id, in characters: a UUID is 36, and 8 rules out accidental short values.
const SESSION_ID_LENGTH: std::ops::RangeInclusive<usize> = 8..=100;
/// Shortest and longest base commit a run update accepts, in hex digits.
/// 7 is git's default abbreviation and 64 a full SHA-256 id. Stricter than `run_check`'s `ACCEPTED_COMMIT_ID_LENGTH`,
/// which must also read older records.
const BASE_COMMIT_LENGTH: std::ops::RangeInclusive<usize> = 7..=64;
/// Ticket reports or run reports one update may submit; matches `MAX_TICKETS`, one report per ticket.
const MAX_REPORTS_PER_UPDATE: usize = 100;
/// Ticket reports or run reports a run may hold in total, so a long run cannot grow its record without bound.
const MAX_REPORTS_PER_RUN: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Run {
    pub(crate) gates: Vec<crate::run_gate::GateResult>,
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) project: String,
    pub(crate) stage: Stage,
    pub(crate) status: RunStatus,
    #[serde(default)]
    pub(crate) activity: Option<String>,
    #[serde(default)]
    pub(crate) current_ticket: Option<String>,
    /// Why the run is blocked; the dashboard shows it as the status reason.
    #[serde(default)]
    pub(crate) blocking_reason: Option<String>,
    /// Number of the current or latest Rework: a return from Check to Build.
    #[serde(default)]
    pub(crate) rework: Option<u32>,
    #[serde(default)]
    pub(crate) updated_at: Option<u64>,
    /// When the agent last updated the run. Unlike `updated_at`, the dashboard's own pause blocks and
    /// resumes leave it alone, so a pause logged after the agent's last update is never mistaken for
    /// older. Records from before it existed fall back to `updated_at`.
    #[serde(default)]
    pub(crate) agent_updated_at: Option<u64>,
    // Run context. Records from before it existed have none of it: it stays unknown, never guessed.
    /// The canonical repository (main checkout), the same for every linked worktree of it.
    #[serde(default)]
    pub(crate) repository: Option<String>,
    #[serde(default)]
    pub(crate) worktree: Option<String>,
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) base_commit: Option<String>,
    #[serde(default)]
    pub(crate) mode: Option<Mode>,
    #[serde(default)]
    pub(crate) spec: Option<String>,
    #[serde(default)]
    pub(crate) pr: Option<String>,
    #[serde(default)]
    pub(crate) tickets: Vec<Ticket>,
    #[serde(default)]
    pub(crate) ticket_reports: Vec<TicketReport>,
    #[serde(default)]
    pub(crate) run_reports: Vec<RunReport>,
    /// One short summary per stage, kept after later stages begin.
    #[serde(default)]
    pub(crate) summaries: BTreeMap<Stage, String>,
    // Token usage. Only identifiers and counts, never log text.
    /// When the stage, rework or active ticket changed, oldest first: how log entries are attributed.
    #[serde(default)]
    pub(crate) timeline: Vec<Mark>,
    /// When the run was blocked and resumed, oldest first. Separate from the timeline, so passes and usage are unaffected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) events: Vec<RunEvent>,
    /// When the run was marked done; usage after that is not the run's.
    #[serde(default)]
    pub(crate) finished_at: Option<u64>,
    /// Ids of the agent sessions recorded for this run.
    #[serde(default)]
    pub(crate) sessions: Vec<String>,
    #[serde(default)]
    pub(crate) usage: Option<Usage>,
    /// The state of the pull request, read from its host by the dashboard server. Callers cannot set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pr_status: Option<PrStatus>,
    /// The model-routing selection resolved when the run started; absent when none applied. A selection
    /// recorded in an earlier shape reads as absent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "routing_or_none"
    )]
    pub(crate) model_routing: Option<ModelRouting>,
}

/// Reads a stored selection, treating only the earlier shape (an object with `roles` or `tasks`) as absent.
/// Any other invalid value is an error.
fn routing_or_none<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ModelRouting>, D::Error> {
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if value
        .as_object()
        .is_some_and(|object| object.contains_key("roles") || object.contains_key("tasks"))
    {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Stage {
    Define,
    Build,
    Check,
    Finish,
}

impl Stage {
    pub(crate) const ALL: [Stage; 4] = [Stage::Define, Stage::Build, Stage::Check, Stage::Finish];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Define => "define",
            Self::Build => "build",
            Self::Check => "check",
            Self::Finish => "finish",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RunStatus {
    Running,
    Blocked,
    Done,
}

/// The workflow skill carrying the run.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Mode {
    #[serde(rename = "jidoka-full")]
    Full,
    #[serde(rename = "jidoka-balanced")]
    Balanced,
    #[serde(rename = "jidoka-fast")]
    Fast,
}

impl Mode {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Full => "jidoka-full",
            Self::Balanced => "jidoka-balanced",
            Self::Fast => "jidoka-fast",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TicketStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct Ticket {
    pub(crate) reference: String,
    pub(crate) title: String,
    pub(crate) status: TicketStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ReportKind {
    Skill,
    Activity,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewRunReport {
    pub(crate) kind: ReportKind,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RunEventKind {
    Blocked,
    Resumed,
}

/// A moment the run became blocked or resumed, with the stage it was in and, when blocked, why.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct RunEvent {
    pub(crate) at: u64,
    pub(crate) kind: RunEventKind,
    pub(crate) stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RunReport {
    pub(crate) kind: ReportKind,
    pub(crate) text: String,
    pub(crate) stage: Stage,
    pub(crate) at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewTicketReport {
    pub(crate) ticket: String,
    pub(crate) kind: ReportKind,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TicketReport {
    pub(crate) ticket: String,
    pub(crate) kind: ReportKind,
    pub(crate) text: String,
    pub(crate) stage: Stage,
    pub(crate) at: u64,
}

/// A ticket as listed by an update; its status is carried over from the stored list or starts pending.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewTicket {
    pub(crate) reference: String,
    pub(crate) title: String,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct NewRun {
    pub(crate) title: String,
    pub(crate) project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) repository: Option<String>,
    /// The agent session the run is started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model_routing: Option<ModelRouting>,
}

impl NewRun {
    fn validate(&self) -> io::Result<()> {
        if self.title.trim().is_empty()
            || self.title.len() > MAX_TEXT_BYTES
            || self.title.chars().any(char::is_control)
        {
            return Err(io_invalid(format!(
                "Title must be nonempty, at most {MAX_TEXT_BYTES} bytes, and contain no control characters"
            )));
        }
        if self.project.trim().is_empty()
            || self.project.len() > MAX_PATH_BYTES
            || self.project.chars().any(char::is_control)
        {
            return Err(io_invalid(format!(
                "Project must be nonempty, at most {MAX_PATH_BYTES} bytes, and contain no control characters"
            )));
        }
        if let Some(session) = &self.session {
            check_session_id(session).map_err(io_invalid)?;
        }
        if let Some(routing) = &self.model_routing {
            routing.validate().map_err(io_invalid)?;
        }
        if self.repository.as_deref().is_some_and(|repository| {
            repository.len() > MAX_PATH_BYTES
                || repository.chars().any(char::is_control)
                || !is_absolute_path(repository)
        }) {
            return Err(io_invalid(format!(
                "Repository must be an absolute path of at most {MAX_PATH_BYTES} bytes without control characters"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateRun {
    pub(crate) stage: Stage,
    pub(crate) status: RunStatus,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) activity: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) current_ticket: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) blocking_reason: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) rework: FieldUpdate<u32>,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) mode: FieldUpdate<Mode>,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) worktree: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) branch: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) base_commit: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) spec: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) pr: FieldUpdate,
    /// The whole ordered ticket list; null removes it.
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) tickets: FieldUpdate<Vec<NewTicket>>,
    /// New statuses by ticket reference, applied after `tickets`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) ticket_status: BTreeMap<String, TicketStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) ticket_reports: Vec<NewTicketReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) run_reports: Vec<NewRunReport>,
    /// Stage summaries to set; null removes that stage's summary. Other stages keep theirs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) summaries: BTreeMap<Stage, Option<String>>,
    /// Agent sessions to attribute to this run, added to the ones already recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sessions: Vec<String>,
    /// The run's whole set of model-routing overrides; null removes them.
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) model_routing: FieldUpdate<BTreeMap<Stage, ModelChoice>>,
}

// Missing fields preserve stored values; null clears them; values replace them.
#[derive(Debug, Default, Serialize)]
#[serde(untagged)]
pub(crate) enum FieldUpdate<T = String> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FieldUpdate<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Set(value),
            None => Self::Clear,
        })
    }
}

impl<T> FieldUpdate<T> {
    pub(crate) fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    pub(crate) fn apply(self, target: &mut Option<T>) {
        match self {
            Self::Keep => {}
            Self::Clear => *target = None,
            Self::Set(value) => *target = Some(value),
        }
    }
}

impl FieldUpdate {
    pub(crate) fn validate(&self, limit: usize) -> Result<(), String> {
        if let Self::Set(value) = self
            && (value.trim().is_empty() || value.len() > limit)
        {
            return Err("Run metadata must be nonblank and within its size limit".into());
        }
        Ok(())
    }
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

/// An absolute `http(s)` URL with a host and no credentials, so a reference never carries a secret.
fn check_http_url(name: &str, value: &str) -> Result<(), String> {
    check_http_url_with(name, value, true)
}

/// A URL that is stored and shown as a reference also may not carry a query string or fragment,
/// which is where tokens usually hide.
fn check_plain_url(name: &str, value: &str) -> Result<(), String> {
    check_http_url_with(name, value, false)
}

fn check_http_url_with(name: &str, value: &str, allow_query: bool) -> Result<(), String> {
    if !allow_query && (value.contains('?') || value.contains('#')) {
        return Err(format!(
            "{name} must not contain a query string or fragment, which can carry secrets"
        ));
    }
    if value != value.trim() || value.chars().any(char::is_whitespace) {
        return Err(format!("{name} must not contain spaces"));
    }
    let uri = rocket::http::uri::Absolute::parse(value).ok();
    let valid = uri.as_ref().is_some_and(|uri| {
        (uri.scheme().eq_ignore_ascii_case("http") || uri.scheme().eq_ignore_ascii_case("https"))
            && uri
                .authority()
                .is_some_and(|authority| !authority.host().is_empty())
    });
    if !valid {
        return Err(format!("{name} must be an absolute HTTP(S) URL"));
    }
    if uri
        .and_then(|uri| uri.authority().and_then(|a| a.user_info().map(|_| ())))
        .is_some()
    {
        return Err(format!("{name} must not contain credentials"));
    }
    Ok(())
}

/// Whether `value` starts like a URL scheme (`javascript:`), not a Windows drive (`C:\`).
fn has_scheme(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    let is_scheme = scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c));
    is_scheme && !rest.is_empty()
}

/// A reference to a document: an `http(s)` URL, or a local path or identifier without a scheme.
fn check_reference(name: &str, value: &str, limit: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > limit {
        return Err(format!("{name} must be nonblank and at most {limit} bytes"));
    }
    if has_control(value) {
        return Err(format!("{name} must not contain control characters"));
    }
    // Leading spaces would hide a URL's scheme from this check yet not from a browser.
    if value != value.trim() {
        return Err(format!("{name} must not start or end with spaces"));
    }
    if value.to_ascii_lowercase().starts_with("http://")
        || value.to_ascii_lowercase().starts_with("https://")
    {
        return check_plain_url(name, value);
    }
    if has_scheme(value) {
        return Err(format!("{name} must be an HTTP(S) URL or a local path"));
    }
    Ok(())
}

fn check_text(name: &str, value: &str, limit: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > limit {
        return Err(format!("{name} must be nonblank and at most {limit} bytes"));
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(format!("{name} must not contain control characters"));
    }
    Ok(())
}

/// Whether `value` is written as an absolute path: `/dir`, `\\server\share` or `C:\dir`. The same
/// answer on every system, because a record may be written on one and read on another.
pub(crate) fn is_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    let windows_drive = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    value.starts_with('/') || value.starts_with("\\\\") || windows_drive
}

/// An agent session id such as a UUID: letters, digits, `-`, `_` and `.`, so it is safe to store.
pub(crate) fn check_session_id(id: &str) -> Result<(), String> {
    let valid = SESSION_ID_LENGTH.contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "A session id must be {} to {} letters, digits, \"-\", \"_\" or \".\"",
            SESSION_ID_LENGTH.start(),
            SESSION_ID_LENGTH.end()
        ))
    }
}

fn io_invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

impl UpdateRun {
    pub(crate) fn new(stage: Stage, status: RunStatus) -> Self {
        Self {
            stage,
            status,
            activity: FieldUpdate::Keep,
            current_ticket: FieldUpdate::Keep,
            blocking_reason: FieldUpdate::Keep,
            rework: FieldUpdate::Keep,
            mode: FieldUpdate::Keep,
            worktree: FieldUpdate::Keep,
            branch: FieldUpdate::Keep,
            base_commit: FieldUpdate::Keep,
            spec: FieldUpdate::Keep,
            pr: FieldUpdate::Keep,
            tickets: FieldUpdate::Keep,
            ticket_status: BTreeMap::new(),
            ticket_reports: Vec::new(),
            run_reports: Vec::new(),
            summaries: BTreeMap::new(),
            sessions: Vec::new(),
            model_routing: FieldUpdate::Keep,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.activity.validate(MAX_NOTE_BYTES)?;
        self.blocking_reason.validate(MAX_NOTE_BYTES)?;
        self.current_ticket.validate(MAX_PATH_BYTES)?;
        if let FieldUpdate::Set(count) = self.rework
            && !(1..=MAX_REWORK).contains(&count)
        {
            return Err("Rework must be a whole number from 1 to 99".into());
        }
        if let FieldUpdate::Set(value) = &self.current_ticket {
            check_http_url("Current ticket", value)?;
        }
        if let FieldUpdate::Set(value) = &self.worktree {
            check_text("Worktree", value, MAX_PATH_BYTES)?;
            if has_control(value) || !is_absolute_path(value) {
                return Err("Worktree must be an absolute path without control characters".into());
            }
        }
        if let FieldUpdate::Set(value) = &self.branch
            && (value.is_empty()
                || value.len() > MAX_BRANCH_BYTES
                || value.starts_with(['-', '/'])
                || value.ends_with(['/', '.'])
                || value.ends_with(".lock")
                || value.contains("..")
                || value.contains("@{")
                || value.contains("//")
                || value
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c)))
        {
            return Err("Branch must be a valid git branch name: no spaces, control characters or any of ~ ^ : ? * [ \\ \"..\" \"@{\", and not starting with \"-\"".into());
        }
        if let FieldUpdate::Set(value) = &self.base_commit
            && (!BASE_COMMIT_LENGTH.contains(&value.len())
                || !value.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(format!(
                "Base commit must be {} to {} hexadecimal characters",
                BASE_COMMIT_LENGTH.start(),
                BASE_COMMIT_LENGTH.end()
            ));
        }
        if let FieldUpdate::Set(value) = &self.spec {
            check_reference("Spec", value, MAX_PATH_BYTES)?;
        }
        if let FieldUpdate::Set(value) = &self.pr {
            check_plain_url("Pull request", value)?;
        }
        if let FieldUpdate::Set(list) = &self.tickets {
            if list.len() > MAX_TICKETS {
                return Err(format!("At most {MAX_TICKETS} tickets can be listed"));
            }
            for (index, ticket) in list.iter().enumerate() {
                check_reference("Ticket reference", &ticket.reference, MAX_PATH_BYTES)?;
                if ticket.reference.chars().any(char::is_whitespace) {
                    return Err(format!(
                        "Ticket reference {:?} must not contain spaces",
                        ticket.reference
                    ));
                }
                check_text("Ticket title", &ticket.title, MAX_TEXT_BYTES)?;
                if list[..index]
                    .iter()
                    .any(|other| other.reference == ticket.reference)
                {
                    return Err(format!("Ticket {} is listed twice", ticket.reference));
                }
            }
        }
        if self.ticket_reports.len() > MAX_REPORTS_PER_UPDATE {
            return Err(format!(
                "At most {MAX_REPORTS_PER_UPDATE} ticket reports can be submitted at once"
            ));
        }
        for report in &self.ticket_reports {
            check_reference("Ticket reference", &report.ticket, MAX_PATH_BYTES)?;
            if report.ticket.chars().any(char::is_whitespace) {
                return Err("Ticket reference must not contain spaces".into());
            }
            check_text("Ticket report", &report.text, MAX_TEXT_BYTES)?;
            if has_control(&report.text) {
                return Err("Ticket report must not contain control characters".into());
            }
        }
        if self.run_reports.len() > MAX_REPORTS_PER_UPDATE {
            return Err(format!(
                "At most {MAX_REPORTS_PER_UPDATE} run reports can be submitted at once"
            ));
        }
        for report in &self.run_reports {
            check_text("Run report", &report.text, MAX_TEXT_BYTES)?;
            if has_control(&report.text) {
                return Err("Run report must not contain control characters".into());
            }
        }
        for summary in self.summaries.values().flatten() {
            check_text("Stage summary", summary, MAX_NOTE_BYTES)?;
        }
        for session in &self.sessions {
            check_session_id(session)?;
        }
        if self.sessions.len() > MAX_SESSIONS {
            return Err(format!(
                "At most {MAX_SESSIONS} sessions can be given at once"
            ));
        }
        Ok(())
    }
}

/// Parses `name=value` where the name may not contain `=`.
fn split_pair<'a>(flag: &str, value: &'a str) -> Result<(&'a str, &'a str), String> {
    value
        .split_once('=')
        .ok_or_else(|| format!("{flag} needs the form <name>=<value>, got {value:?}"))
}

pub(crate) fn parse_update(
    stage: &str,
    status: &str,
    options: &[&str],
) -> Result<UpdateRun, String> {
    let mut update = UpdateRun::new(
        serde_json::from_value(stage.into()).map_err(|e| e.to_string())?,
        serde_json::from_value(status.into()).map_err(|e| e.to_string())?,
    );
    let mut tickets: Vec<NewTicket> = Vec::new();
    let mut overrides: BTreeMap<Stage, ModelChoice> = BTreeMap::new();
    let mut args = options.iter();
    while let Some(flag) = args.next() {
        let mut next_value = || {
            args.next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("Missing value for {flag}\n{UPDATE_HELP}"))
        };
        let repeated = || format!("Repeated or conflicting run update option: {flag}");
        match *flag {
            "--rework" | "--clear-rework" => {
                if !update.rework.is_keep() {
                    return Err(repeated());
                }
                update.rework = if *flag == "--clear-rework" {
                    FieldUpdate::Clear
                } else {
                    let count = next_value()?;
                    FieldUpdate::Set(count.parse().map_err(|_| {
                        format!("Invalid value for --rework: {count}; use a whole number from 1 to {MAX_REWORK}")
                    })?)
                };
                continue;
            }
            "--mode" | "--clear-mode" => {
                if !update.mode.is_keep() {
                    return Err(repeated());
                }
                update.mode = if *flag == "--clear-mode" {
                    FieldUpdate::Clear
                } else {
                    let mode = next_value()?;
                    FieldUpdate::Set(serde_json::from_value(serde_json::Value::from(*mode)).map_err(|_| {
                        format!("Invalid value for --mode: {mode}; use jidoka-full, jidoka-balanced or jidoka-fast")
                    })?)
                };
                continue;
            }
            "--ticket" => {
                if !update.tickets.is_keep() && tickets.is_empty() {
                    return Err(repeated());
                }
                let value = next_value()?;
                let (reference, title) = value
                    .split_once(char::is_whitespace)
                    .map(|(reference, title)| (reference, title.trim()))
                    .filter(|(_, title)| !title.is_empty())
                    .ok_or_else(|| {
                        format!("--ticket needs \"<reference> <title>\", got {value:?}")
                    })?;
                tickets.push(NewTicket {
                    reference: reference.to_string(),
                    title: title.to_string(),
                });
                continue;
            }
            "--clear-tickets" => {
                if !update.tickets.is_keep() || !tickets.is_empty() {
                    return Err(repeated());
                }
                update.tickets = FieldUpdate::Clear;
                continue;
            }
            "--model-routing" => {
                if !update.model_routing.is_keep() {
                    return Err(repeated());
                }
                let value = next_value()?;
                let (name, choice) = split_pair(flag, value)?;
                let stage: Stage = serde_json::from_value(name.into()).map_err(|_| {
                    format!("Unknown stage {name:?} for --model-routing; use define, build, check or finish")
                })?;
                let choice = ModelChoice::parse(choice)
                    .map_err(|problem| format!("Invalid --model-routing for {name}: {problem}"))?;
                if overrides.insert(stage, choice).is_some() {
                    return Err(format!("Model for {name} given twice"));
                }
                continue;
            }
            "--clear-model-routing" => {
                if !update.model_routing.is_keep() || !overrides.is_empty() {
                    return Err(repeated());
                }
                update.model_routing = FieldUpdate::Clear;
                continue;
            }
            "--session" => {
                let id = next_value()?;
                check_session_id(id)?;
                if !update.sessions.iter().any(|known| known == *id) {
                    update.sessions.push((*id).to_string());
                }
                continue;
            }
            "--run-report" => {
                let kind = *next_value()?;
                let text = *next_value()?;
                let kind = serde_json::from_value(kind.into())
                    .map_err(|_| "Run report kind must be skill or activity")?;
                update.run_reports.push(NewRunReport {
                    kind,
                    text: text.to_string(),
                });
                continue;
            }
            "--ticket-report" => {
                let reference = *next_value()?;
                let kind = *next_value()?;
                let text = *next_value()?;
                let kind = serde_json::from_value(kind.into())
                    .map_err(|_| "Ticket report kind must be skill or activity")?;
                update.ticket_reports.push(NewTicketReport {
                    ticket: reference.to_string(),
                    kind,
                    text: text.to_string(),
                });
                continue;
            }
            "--ticket-status" => {
                let value = next_value()?;
                let (reference, status) = value.rsplit_once('=').ok_or_else(|| {
                    format!("--ticket-status needs <reference>=<status>, got {value:?}")
                })?;
                let status: TicketStatus = serde_json::from_value(status.into()).map_err(|_| {
                    format!("Invalid ticket status {status:?}; use pending, in-progress or done")
                })?;
                if update
                    .ticket_status
                    .insert(reference.to_string(), status)
                    .is_some()
                {
                    return Err(format!("Ticket status for {reference} given twice"));
                }
                continue;
            }
            "--summary" | "--clear-summary" => {
                let value = next_value()?;
                let (name, text) = if *flag == "--summary" {
                    let (name, text) = split_pair(flag, value)?;
                    (name, Some(text.to_string()))
                } else {
                    (*value, None)
                };
                let stage: Stage = serde_json::from_value(name.into()).map_err(|_| {
                    format!("Unknown stage {name:?} for {flag}; use define, build, check or finish")
                })?;
                if update.summaries.insert(stage, text).is_some() {
                    return Err(format!("Summary for {name} given twice"));
                }
                continue;
            }
            _ => {}
        }
        let field = match *flag {
            "--activity" | "--clear-activity" => &mut update.activity,
            "--current-ticket" | "--clear-current-ticket" => &mut update.current_ticket,
            "--blocking-reason" | "--clear-blocking-reason" => &mut update.blocking_reason,
            "--worktree" | "--clear-worktree" => &mut update.worktree,
            "--branch" | "--clear-branch" => &mut update.branch,
            "--base-commit" | "--clear-base-commit" => &mut update.base_commit,
            "--spec" | "--clear-spec" => &mut update.spec,
            "--pr" | "--clear-pr" => &mut update.pr,
            _ => return Err(format!("Unknown run update option: {flag}\n{UPDATE_HELP}")),
        };
        if !field.is_keep() {
            return Err(repeated());
        }
        *field = if flag.starts_with("--clear-") {
            FieldUpdate::Clear
        } else {
            FieldUpdate::Set((*next_value()?).to_string())
        };
    }
    if !tickets.is_empty() {
        update.tickets = FieldUpdate::Set(tickets);
    }
    if !overrides.is_empty() {
        update.model_routing = FieldUpdate::Set(overrides);
    }
    update.validate()?;
    Ok(update)
}

pub(crate) fn updated_now() -> io::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    u64::try_from(millis).map_err(io::Error::other)
}

/// The run records, shared by clones: usage is collected in the background on a clone.
#[derive(Clone)]
pub(crate) struct RunStore {
    shared: Arc<Shared>,
}

struct Shared {
    path: PathBuf,
    runs: Mutex<Vec<Run>>,
    collection: Collection,
}

#[derive(Debug)]
pub(crate) struct UpdateOutcome {
    pub(crate) run: Run,
    /// Notes on an update that was accepted but looks wrong; each is one line telling the agent what to do.
    pub(crate) warnings: Vec<String>,
}

/// Warns, without refusing, when an update skips a stage or finishes Build without a spec.
fn update_warnings(previous: Stage, next: &Run) -> Vec<String> {
    let mut warnings = Vec::new();
    if next.status == RunStatus::Done && previous != next.stage {
        warnings.push(format!(
            "Stage {} was set to done but never entered (the run was in {}); set each stage to running first.",
            next.stage.name(),
            previous.name()
        ));
    }
    let leaves_build = (next.stage == Stage::Build && next.status == RunStatus::Done)
        || (next.stage == Stage::Check && previous != Stage::Check);
    if leaves_build && next.mode == Some(Mode::Full) && next.spec.is_none() {
        warnings.push(
            "This Full run has no spec recorded; record it with --spec <url|path>.".to_string(),
        );
    }
    warnings
}

impl RunStore {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            shared: Arc::new(Shared {
                path: path.to_owned(),
                runs: Mutex::new(load_runs(path)?),
                collection: Collection::default(),
            }),
        })
    }

    pub(crate) fn usage_collection(&self) -> &Collection {
        &self.shared.collection
    }

    pub(crate) fn list(&self) -> io::Result<Vec<Run>> {
        Ok(self.shared.runs.lock().map_err(lock_error)?.clone())
    }

    pub(crate) fn get(&self, id: &str) -> io::Result<Option<Run>> {
        Ok(self
            .shared
            .runs
            .lock()
            .map_err(lock_error)?
            .iter()
            .find(|run| run.id == id)
            .cloned())
    }

    pub(crate) fn create(&self, new: NewRun) -> io::Result<Run> {
        new.validate()?;
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let mut id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        while runs.iter().any(|run| run.id == id.to_string()) {
            id += 1;
        }
        let created = updated_now()?;
        let run = Run {
            gates: Vec::new(),
            id: id.to_string(),
            title: new.title,
            project: new.project,
            stage: Stage::Define,
            status: RunStatus::Running,
            activity: None,
            current_ticket: None,
            blocking_reason: None,
            rework: None,
            updated_at: Some(created),
            agent_updated_at: Some(created),
            repository: new.repository,
            worktree: None,
            branch: None,
            base_commit: None,
            mode: None,
            spec: None,
            pr: None,
            tickets: Vec::new(),
            ticket_reports: Vec::new(),
            run_reports: Vec::new(),
            summaries: BTreeMap::new(),
            timeline: Vec::new(),
            events: Vec::new(),
            finished_at: None,
            sessions: new.session.into_iter().collect(),
            usage: None,
            pr_status: None,
            model_routing: new.model_routing,
        };
        let mut run = run;
        let created = run.updated_at.unwrap_or(0);
        mark(&mut run, created, None);
        runs.push(run.clone());
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs.pop();
            return Err(error);
        }
        Ok(run)
    }

    /// Applies an update to a copy first, so an inconsistent update changes nothing, and
    /// restores the stored record when saving fails.
    pub(crate) fn update(&self, id: &str, update: UpdateRun) -> io::Result<Option<UpdateOutcome>> {
        update.validate().map_err(io_invalid)?;
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(None);
        };
        let updated_at = updated_now()?;
        let mut next = runs[index].clone();
        let was_blocked = matches!(next.status, RunStatus::Blocked);
        let previous_stage = next.stage;
        next.stage = update.stage;
        next.status = update.status;
        let started_with = match &update.activity {
            FieldUpdate::Set(text) => Some(text.clone()),
            _ => None,
        };
        update.activity.apply(&mut next.activity);
        update.current_ticket.apply(&mut next.current_ticket);
        update.blocking_reason.apply(&mut next.blocking_reason);
        update.rework.apply(&mut next.rework);
        update.mode.apply(&mut next.mode);
        update.worktree.apply(&mut next.worktree);
        update.branch.apply(&mut next.branch);
        update.base_commit.map_hex().apply(&mut next.base_commit);
        update.spec.apply(&mut next.spec);
        update.pr.apply(&mut next.pr);
        let warnings = update_warnings(previous_stage, &next);
        match update.tickets {
            FieldUpdate::Keep => {}
            FieldUpdate::Clear => next.tickets.clear(),
            FieldUpdate::Set(list) => {
                let old = std::mem::take(&mut next.tickets);
                next.tickets = list
                    .into_iter()
                    .map(|ticket| Ticket {
                        status: old
                            .iter()
                            .find(|kept| kept.reference == ticket.reference)
                            .map_or(TicketStatus::Pending, |kept| kept.status),
                        reference: ticket.reference,
                        title: ticket.title,
                    })
                    .collect();
            }
        }
        for (reference, status) in update.ticket_status {
            let ticket = next
                .tickets
                .iter_mut()
                .find(|ticket| ticket.reference == reference)
                .ok_or_else(|| {
                    io_invalid(format!(
                        "No ticket {reference} is listed on this run; list it with --ticket first"
                    ))
                })?;
            ticket.status = status;
        }
        if next
            .tickets
            .iter()
            .filter(|t| t.status == TicketStatus::InProgress)
            .count()
            > 1
        {
            return Err(io_invalid("Only one ticket can be in progress at a time"));
        }
        if next.ticket_reports.len() + update.ticket_reports.len() > MAX_REPORTS_PER_RUN {
            return Err(io_invalid(format!(
                "A run can hold at most {MAX_REPORTS_PER_RUN} ticket reports"
            )));
        }
        for report in update.ticket_reports {
            if !next
                .tickets
                .iter()
                .any(|ticket| ticket.reference == report.ticket)
            {
                return Err(io_invalid(format!(
                    "No ticket {} is listed on this run",
                    report.ticket
                )));
            }
            next.ticket_reports.push(TicketReport {
                ticket: report.ticket,
                kind: report.kind,
                text: report.text,
                stage: update.stage,
                at: updated_at,
            });
        }
        if next.run_reports.len() + update.run_reports.len() > MAX_REPORTS_PER_RUN {
            return Err(io_invalid(format!(
                "A run can hold at most {MAX_REPORTS_PER_RUN} run reports"
            )));
        }
        for report in update.run_reports {
            next.run_reports.push(RunReport {
                kind: report.kind,
                text: report.text,
                stage: update.stage,
                at: updated_at,
            });
        }
        match update.model_routing {
            FieldUpdate::Keep => {}
            FieldUpdate::Clear => {
                if let Some(routing) = next.model_routing.as_mut() {
                    routing.overrides.clear();
                }
            }
            FieldUpdate::Set(overrides) => {
                let routing = next.model_routing.as_mut().ok_or_else(|| {
                    io_invalid("This run has no model routing selection to override")
                })?;
                routing.overrides = overrides;
                routing.validate().map_err(io_invalid)?;
            }
        }
        for (stage, summary) in update.summaries {
            match summary {
                Some(text) => {
                    next.summaries.insert(stage, text);
                }
                None => {
                    next.summaries.remove(&stage);
                }
            }
        }
        if update.stage == Stage::Finish
            && update.status == RunStatus::Done
            && next
                .summaries
                .get(&Stage::Finish)
                .is_none_or(|text| text.trim().is_empty())
        {
            return Err(io_invalid(
                "Finish cannot be marked done without a Finish summary; add --summary finish=<what shipped>",
            ));
        }
        for session in update.sessions {
            if !next.sessions.contains(&session) {
                next.sessions.push(session);
            }
        }
        if next.sessions.len() > KEEP_SESSIONS {
            next.sessions.drain(..next.sessions.len() - KEEP_SESSIONS);
        }
        next.updated_at = Some(updated_at);
        next.agent_updated_at = Some(updated_at);
        let is_blocked = matches!(next.status, RunStatus::Blocked);
        if was_blocked != is_blocked && next.events.len() < MAX_EVENTS {
            next.events.push(RunEvent {
                at: updated_at,
                kind: if is_blocked {
                    RunEventKind::Blocked
                } else {
                    RunEventKind::Resumed
                },
                stage: next.stage,
                reason: if is_blocked {
                    next.blocking_reason.clone()
                } else {
                    None
                },
            });
        }
        // Where the work is now, for attributing token usage; and when it ended.
        mark(&mut next, updated_at, started_with);
        next.finished_at = match (next.status, next.finished_at) {
            (RunStatus::Done, None) => Some(updated_at),
            (RunStatus::Done, kept) => kept,
            _ => None,
        };
        let old = std::mem::replace(&mut runs[index], next);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs[index] = old;
            return Err(error);
        }
        Ok(Some(UpdateOutcome {
            run: runs[index].clone(),
            warnings,
        }))
    }

    /// Atomically change a run and persist it, restoring the old value on save failure.
    pub(crate) fn change_gate(
        &self,
        id: &str,
        change: impl FnOnce(&mut Run) -> Result<(), String>,
    ) -> Result<(), String> {
        let original = self
            .get(id)
            .map_err(|e| format!("Could not read the run: {e}"))?
            .ok_or_else(|| format!("Run {id} not found"))?;
        let snapshot = serde_json::to_vec(&original).map_err(|e| e.to_string())?;
        let mut next = original;
        change(&mut next)?;
        let now = updated_now().map_err(|e| format!("Could not read the time: {e}"))?;
        next.updated_at = Some(now);
        next.agent_updated_at = Some(now);
        mark(&mut next, now, None);
        if next.status == RunStatus::Done {
            next.finished_at = Some(now);
        }
        let mut runs = self.shared.runs.lock().map_err(|e| e.to_string())?;
        let index = runs
            .iter()
            .position(|run| run.id == id)
            .ok_or_else(|| format!("Run {id} not found"))?;
        if serde_json::to_vec(&runs[index]).map_err(|e| e.to_string())? != snapshot {
            return Err("The run changed while checking gates; try again".into());
        }
        let old = std::mem::replace(&mut runs[index], next);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs[index] = old;
            return Err(format!("Could not save the run: {error}"));
        }
        Ok(())
    }

    /// Brings the run in line with the latest pause its log shows (`None` when it shows none). A running
    /// run is blocked for an unresumed pause; a run this method blocked takes the reason of the current
    /// pause, and is set running again once the log shows later activity or no pause at all. Only a state
    /// the dashboard itself set is changed: never a block or reason the agent set, a done run, or a run the
    /// agent updated after the pause began.
    pub(crate) fn apply_pause(&self, id: &str, latest: Option<&Pause>) -> io::Result<()> {
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(());
        };
        let mut next = runs[index].clone();
        let agent_at = next.agent_updated_at.or(next.updated_at);
        let automatic = is_automatic_reason(next.status, next.blocking_reason.as_deref());
        let now = updated_now()?;
        let (kind, at) = match (next.status, latest) {
            (
                RunStatus::Running,
                Some(
                    pause @ Pause {
                        resumed_at: None, ..
                    },
                ),
            ) if Some(pause.at) >= agent_at => {
                next.status = RunStatus::Blocked;
                next.blocking_reason = Some(pause.kind.reason().to_string());
                (RunEventKind::Blocked, pause.at)
            }
            (
                RunStatus::Blocked,
                Some(
                    pause @ Pause {
                        resumed_at: None, ..
                    },
                ),
            ) if automatic && next.blocking_reason.as_deref() != Some(pause.kind.reason()) => {
                next.blocking_reason = Some(pause.kind.reason().to_string());
                (RunEventKind::Blocked, pause.at)
            }
            (
                RunStatus::Blocked,
                Some(Pause {
                    resumed_at: Some(resumed),
                    ..
                }),
            ) if automatic => {
                next.status = RunStatus::Running;
                next.blocking_reason = None;
                (RunEventKind::Resumed, *resumed)
            }
            (RunStatus::Blocked, None) if automatic => {
                next.status = RunStatus::Running;
                next.blocking_reason = None;
                (RunEventKind::Resumed, now)
            }
            _ => return Ok(()),
        };
        if next.events.len() < MAX_EVENTS {
            next.events.push(RunEvent {
                at,
                kind,
                stage: next.stage,
                reason: next.blocking_reason.clone(),
            });
        }
        next.agent_updated_at = agent_at;
        next.updated_at = Some(now);
        let old = std::mem::replace(&mut runs[index], next);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs[index] = old;
            return Err(error);
        }
        Ok(())
    }

    /// Replaces the usage of a run. A failed save leaves the stored usage as it was.
    pub(crate) fn set_usage(&self, id: &str, usage: Usage) -> io::Result<()> {
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(run) = runs.iter_mut().find(|run| run.id == id) else {
            return Ok(());
        };
        let old = run.usage.replace(usage);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            if let Some(run) = runs.iter_mut().find(|run| run.id == id) {
                run.usage = old;
            }
            return Err(error);
        }
        Ok(())
    }

    /// Stores the state of a Run's pull request, only while the Run still records that pull request: an update
    /// that changed it in the meantime wins. A failed save leaves the stored state as it was.
    pub(crate) fn set_pr_status(&self, id: &str, status: PrStatus) -> io::Result<()> {
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(run) = runs.iter_mut().find(|run| run.id == id) else {
            return Ok(());
        };
        if run.pr.as_deref() != Some(status.url.as_str()) {
            return Ok(());
        }
        let old = run.pr_status.replace(status);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            if let Some(run) = runs.iter_mut().find(|run| run.id == id) {
                run.pr_status = old;
            }
            return Err(error);
        }
        Ok(())
    }

    /// Removes only the Run record. Returns whether a Run with this id existed.
    pub(crate) fn delete(&self, id: &str) -> io::Result<bool> {
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(false);
        };
        let removed = runs.remove(index);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs.insert(index, removed);
            return Err(error);
        }
        Ok(true)
    }
}

impl FieldUpdate {
    /// Commit ids are stored in lowercase, whichever case they were given in.
    fn map_hex(self) -> Self {
        match self {
            Self::Set(value) => Self::Set(value.to_ascii_lowercase()),
            other => other,
        }
    }
}

/// Whether a run is blocked for a reason the dashboard sets for a pause, not one the agent gave.
pub(crate) fn is_automatic_reason(status: RunStatus, reason: Option<&str>) -> bool {
    status == RunStatus::Blocked
        && reason.is_some_and(|reason| PauseKind::from_reason(reason).is_some())
}

/// Appends a mark when the stage, rework or ticket in progress differs from the last one. The mark keeps
/// the activity given with the update that created it.
fn mark(run: &mut Run, at: u64, activity: Option<String>) {
    let stage = run.stage;
    let building = matches!(stage, Stage::Build);
    let current = Mark {
        at,
        stage,
        rework: if building { run.rework } else { None },
        ticket: if building {
            run.tickets
                .iter()
                .find(|t| t.status == TicketStatus::InProgress)
                .map(|t| t.reference.clone())
        } else {
            None
        },
        activity,
    };
    let changed = run.timeline.last().is_none_or(|last| {
        (last.stage, last.rework, &last.ticket) != (current.stage, current.rework, &current.ticket)
    });
    if changed && run.timeline.len() < MAX_MARKS {
        run.timeline.push(current);
    }
}

pub(crate) fn lock_error<T>(_: std::sync::PoisonError<T>) -> io::Error {
    io::Error::other("run storage lock was poisoned")
}

pub(crate) fn load_runs(path: &Path) -> io::Result<Vec<Run>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

pub(crate) fn save_runs(path: &Path, runs: &[Run]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("run storage has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        // Flushed to disk before the rename, so a crash leaves the old or the new file, never a torn one.
        let mut file = fs::File::create(&temp)?;
        file.write_all(&serde_json::to_vec(runs)?)?;
        file.sync_all()?;
    }
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests;
