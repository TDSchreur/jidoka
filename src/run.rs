use crate::pr_state::PrStatus;
use crate::routing::ModelRouting;
use crate::stages::{self, Destination};
use crate::usage::{Collection, Mark, Pause, PauseKind, Usage};
use findings::{FindingsContext, FindingsUpdate};
use model_routing::RoutingUpdate;
use reports::ReportsUpdate;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
pub(crate) use store::RunStore;
use summaries::SummariesUpdate;
use tickets::TicketsUpdate;

mod findings;
mod model_routing;
mod reports;
mod store;
mod summaries;
mod tickets;

pub(crate) const UPDATE_HELP: &str = concat!(
    "Run update options (the stage never changes here; use `jidoka run advance <id>`, or `--force --reason \"<text>\"` when a gate cannot be met):\n  --activity <text> | --clear-activity\n  --current-ticket <reference> | --clear-current-ticket\n  --blocking-reason <text> | --clear-blocking-reason\n  --rework <1-99> | --clear-rework (on a running run in Check, --rework <n> also moves it back to Build for a Rework and needs a new --finding in the same update; elsewhere it only sets the count)\n  --mode <full|balanced|fast|jidoka-full|jidoka-balanced|jidoka-fast> | --clear-mode\n  --worktree <absolute-path> | --clear-worktree\n  --branch <name> | --clear-branch\n  --base-commit <sha> | --clear-base-commit\n  --spec <http(s)-url|path> | --clear-spec\n  --pr <http(s)-url> | --clear-pr\n",
    tickets::help!(),
    findings::help!(),
    reports::help!(),
    summaries::help!(),
    "  --session <agent-session-id> (repeat; attributes that session's token usage to this run)\n",
    model_routing::help!(),
    "Omitted fields are preserved; clear flags remove them."
);
/// Highest Rework count a run can report.
pub(crate) const MAX_REWORK: u32 = 99;
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
pub(crate) const MAX_TEXT_BYTES: usize = 500;
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
/// Ticket reports or run reports a run may hold in total, so a long run cannot grow its record without bound.
pub(crate) const MAX_REPORTS_PER_RUN: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Run {
    /// In-memory generation of changes that invalidate gate validation.
    #[serde(skip)]
    gate_generation: u64,
    pub(crate) gates: Vec<crate::run_gate::GateResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) check_history: Vec<crate::run_gate::CheckSnapshot>,
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
    /// Issues the checks reported, with their outcomes, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) findings: Vec<Finding>,
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
    #[serde(rename = "jidoka-full", alias = "full")]
    Full,
    #[serde(rename = "jidoka-balanced", alias = "balanced")]
    Balanced,
    #[serde(rename = "jidoka-fast", alias = "fast")]
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
pub(crate) struct TicketReport {
    pub(crate) ticket: String,
    pub(crate) kind: ReportKind,
    pub(crate) text: String,
    pub(crate) stage: Stage,
    pub(crate) at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FindingState {
    Open,
    Fixed,
    Dismissed,
    Deferred,
}

/// One issue a check reported, recorded with its outcome.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Finding {
    /// `F1`, `F2`, ... per run.
    pub(crate) id: String,
    pub(crate) check: String,
    pub(crate) location: Option<String>,
    pub(crate) text: String,
    /// The worktree HEAD when the finding was recorded.
    pub(crate) reviewed_commit: String,
    /// The Rework this finding sent the run back to, if it did.
    pub(crate) rework: Option<u32>,
    pub(crate) state: FindingState,
    /// Why it was dismissed or deferred.
    pub(crate) reason: Option<String>,
    /// The worktree HEAD when it was marked fixed.
    pub(crate) fix_commit: Option<String>,
    pub(crate) at: u64,
    /// When its state was last set to something other than open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resolved_at: Option<u64>,
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
    #[serde(flatten)]
    pub(crate) tickets: TicketsUpdate,
    #[serde(flatten)]
    pub(crate) reports: ReportsUpdate,
    #[serde(flatten)]
    pub(crate) findings: FindingsUpdate,
    #[serde(flatten)]
    pub(crate) summaries: SummariesUpdate,
    /// Agent sessions to attribute to this run, added to the ones already recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sessions: Vec<String>,
    #[serde(flatten)]
    pub(crate) routing: RoutingUpdate,
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
    pub(crate) fn new(status: RunStatus) -> Self {
        Self {
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
            tickets: TicketsUpdate::default(),
            reports: ReportsUpdate::default(),
            findings: FindingsUpdate::default(),
            summaries: SummariesUpdate::default(),
            sessions: Vec::new(),
            routing: RoutingUpdate::default(),
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
            check_reference("Current ticket", value, MAX_PATH_BYTES)?;
            if value.chars().any(char::is_whitespace) {
                return Err("Current ticket reference must not contain spaces".into());
            }
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
        self.tickets.validate()?;
        self.reports.validate()?;
        self.findings.validate()?;
        self.summaries.validate()?;
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

fn repeated(flag: &str) -> String {
    format!("Repeated or conflicting run update option: {flag}")
}

pub(crate) fn parse_update(status: &str, options: &[&str]) -> Result<UpdateRun, String> {
    let mut update =
        UpdateRun::new(serde_json::from_value(status.into()).map_err(|e| e.to_string())?);
    let mut args = options.iter();
    while let Some(flag) = args.next() {
        let mut next_value = || {
            args.next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("Missing value for {flag}\n{UPDATE_HELP}"))
        };
        match *flag {
            "--rework" | "--clear-rework" => {
                if !update.rework.is_keep() {
                    return Err(repeated(flag));
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
                    return Err(repeated(flag));
                }
                update.mode = if *flag == "--clear-mode" {
                    FieldUpdate::Clear
                } else {
                    let mode = next_value()?;
                    FieldUpdate::Set(serde_json::from_value(serde_json::Value::from(*mode)).map_err(|_| {
                        format!("Invalid value for --mode: {mode}; use full, balanced or fast (also accepted: jidoka-full, jidoka-balanced, jidoka-fast)")
                    })?)
                };
                continue;
            }
            "--ticket" | "--clear-tickets" | "--ticket-status" => {
                update.tickets.parse_option(flag, &mut next_value)?;
                continue;
            }
            "--model-routing" | "--clear-model-routing" => {
                update.routing.parse_option(flag, &mut next_value)?;
                continue;
            }
            "--run-report" | "--ticket-report" => {
                update.reports.parse_option(flag, &mut next_value)?;
                continue;
            }
            "--summary" | "--clear-summary" => {
                update.summaries.parse_option(flag, &mut next_value)?;
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
            "--finding" | "--finding-status" => {
                update.findings.parse_option(flag, &mut next_value)?;
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
            return Err(repeated(flag));
        }
        *field = if flag.starts_with("--clear-") {
            FieldUpdate::Clear
        } else {
            FieldUpdate::Set((*next_value()?).to_string())
        };
    }
    update.validate()?;
    Ok(update)
}

/// Logs a Blocked or Resumed event when the update changed whether the run is blocked.
fn record_block_change(run: &mut Run, was_blocked: bool, at: u64) {
    let is_blocked = matches!(run.status, RunStatus::Blocked);
    if was_blocked == is_blocked || run.events.len() >= MAX_EVENTS {
        return;
    }
    run.events.push(RunEvent {
        at,
        kind: if is_blocked {
            RunEventKind::Blocked
        } else {
            RunEventKind::Resumed
        },
        stage: run.stage,
        reason: if is_blocked {
            run.blocking_reason.clone()
        } else {
            None
        },
    });
}

pub(crate) fn updated_now() -> io::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    u64::try_from(millis).map_err(io::Error::other)
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
