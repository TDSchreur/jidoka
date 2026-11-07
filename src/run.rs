use crate::pr_state::PrStatus;
use crate::routing::{ModelRouting, Role, Task};
use crate::usage::{Collection, Mark, Usage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const UPDATE_HELP: &str = "Run update options:\n  --activity <text> | --clear-activity\n  --current-ticket <http(s)-url> | --clear-current-ticket\n  --blocking-reason <text> | --clear-blocking-reason\n  --rework <1-99> | --clear-rework\n  --mode <factory-full|factory-balanced|factory-fast> | --clear-mode\n  --worktree <absolute-path> | --clear-worktree\n  --branch <name> | --clear-branch\n  --base-commit <sha> | --clear-base-commit\n  --spec <http(s)-url|path> | --clear-spec\n  --pr <http(s)-url> | --clear-pr\n  --ticket \"<reference> <title>\" (repeat; the flags are the whole ordered list) | --clear-tickets\n  --ticket-status <reference>=<pending|in-progress|done> (repeat)\n  --ticket-report <reference>=<skill|activity>:<text> (repeat)\n  --summary <define|build|check|finish>=<text> | --clear-summary <stage> (repeat)\n  --session <agent-session-id> (repeat; attributes that session's token usage to this run)\n  --model-routing <build|check|finish>=<strong|coding|routine> (repeat; the flags are all of the run's overrides) | --clear-model-routing\nOmitted fields are preserved; clear flags remove them.";
/// Highest Rework count a run can report.
pub(crate) const MAX_REWORK: u32 = 99;
const MAX_TICKETS: usize = 100;
const MAX_SESSIONS: usize = 20;
/// Sessions kept per run, and marks kept on its timeline.
const KEEP_SESSIONS: usize = 100;
pub(crate) const MAX_MARKS: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Run {
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
    /// One short summary per stage, kept after later stages begin.
    #[serde(default)]
    pub(crate) summaries: BTreeMap<Stage, String>,
    // Token usage. Only identifiers and counts, never log text.
    /// When the stage, rework or active ticket changed, oldest first: how log entries are attributed.
    #[serde(default)]
    pub(crate) timeline: Vec<Mark>,
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
    /// The model-routing selection resolved when the run started; absent when none applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model_routing: Option<ModelRouting>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Stage {
    Define,
    Build,
    Check,
    Finish,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
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
    #[serde(rename = "factory-full", alias = "factory")]
    Full,
    #[serde(rename = "factory-balanced")]
    Balanced,
    #[serde(rename = "factory-fast")]
    Fast,
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
            || self.title.len() > 500
            || self.title.chars().any(char::is_control)
        {
            return Err(io_invalid(
                "Title must be nonempty, at most 500 bytes, and contain no control characters",
            ));
        }
        if self.project.trim().is_empty()
            || self.project.len() > 4096
            || self.project.chars().any(char::is_control)
        {
            return Err(io_invalid(
                "Project must be nonempty, at most 4096 bytes, and contain no control characters",
            ));
        }
        if let Some(session) = &self.session {
            check_session_id(session).map_err(io_invalid)?;
        }
        if let Some(routing) = &self.model_routing {
            routing.validate().map_err(io_invalid)?;
        }
        if self.repository.as_deref().is_some_and(|repository| {
            repository.len() > 4096
                || repository.chars().any(char::is_control)
                || !is_absolute_path(repository)
        }) {
            return Err(io_invalid("Repository must be an absolute path of at most 4096 bytes without control characters"));
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
    /// Stage summaries to set; null removes that stage's summary. Other stages keep theirs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) summaries: BTreeMap<Stage, Option<String>>,
    /// Agent sessions to attribute to this run, added to the ones already recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sessions: Vec<String>,
    /// The run's whole set of model-routing overrides; null removes them.
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) model_routing: FieldUpdate<BTreeMap<Task, Role>>,
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
        if let Self::Set(value) = self {
            if value.trim().is_empty() || value.len() > limit {
                return Err("Run metadata must be nonblank and within its size limit".into());
            }
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
    let valid = (8..=100).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if valid {
        Ok(())
    } else {
        Err("A session id must be 8 to 100 letters, digits, \"-\", \"_\" or \".\"".into())
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
            summaries: BTreeMap::new(),
            sessions: Vec::new(),
            model_routing: FieldUpdate::Keep,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.activity.validate(2000)?;
        self.blocking_reason.validate(2000)?;
        self.current_ticket.validate(4096)?;
        if let FieldUpdate::Set(count) = self.rework {
            if !(1..=MAX_REWORK).contains(&count) {
                return Err("Rework must be a whole number from 1 to 99".into());
            }
        }
        if let FieldUpdate::Set(value) = &self.current_ticket {
            check_http_url("Current ticket", value)?;
        }
        if let FieldUpdate::Set(value) = &self.worktree {
            check_text("Worktree", value, 4096)?;
            if has_control(value) || !is_absolute_path(value) {
                return Err("Worktree must be an absolute path without control characters".into());
            }
        }
        if let FieldUpdate::Set(value) = &self.branch {
            if value.is_empty()
                || value.len() > 255
                || value.starts_with(['-', '/'])
                || value.ends_with(['/', '.'])
                || value.ends_with(".lock")
                || value.contains("..")
                || value.contains("@{")
                || value.contains("//")
                || value
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
            {
                return Err("Branch must be a valid git branch name: no spaces, control characters or any of ~ ^ : ? * [ \\ \"..\" \"@{\", and not starting with \"-\"".into());
            }
        }
        if let FieldUpdate::Set(value) = &self.base_commit {
            if !(7..=64).contains(&value.len()) || !value.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("Base commit must be 7 to 64 hexadecimal characters".into());
            }
        }
        if let FieldUpdate::Set(value) = &self.spec {
            check_reference("Spec", value, 4096)?;
        }
        if let FieldUpdate::Set(value) = &self.pr {
            check_plain_url("Pull request", value)?;
        }
        if let FieldUpdate::Set(list) = &self.tickets {
            if list.len() > MAX_TICKETS {
                return Err(format!("At most {MAX_TICKETS} tickets can be listed"));
            }
            for (index, ticket) in list.iter().enumerate() {
                check_reference("Ticket reference", &ticket.reference, 4096)?;
                if ticket.reference.chars().any(char::is_whitespace) {
                    return Err(format!(
                        "Ticket reference {:?} must not contain spaces",
                        ticket.reference
                    ));
                }
                check_text("Ticket title", &ticket.title, 500)?;
                if list[..index]
                    .iter()
                    .any(|other| other.reference == ticket.reference)
                {
                    return Err(format!("Ticket {} is listed twice", ticket.reference));
                }
            }
        }
        if self.ticket_reports.len() > 100 {
            return Err("At most 100 ticket reports can be submitted at once".into());
        }
        for report in &self.ticket_reports {
            check_reference("Ticket reference", &report.ticket, 4096)?;
            if report.ticket.chars().any(char::is_whitespace) {
                return Err("Ticket reference must not contain spaces".into());
            }
            check_text("Ticket report", &report.text, 500)?;
            if has_control(&report.text) {
                return Err("Ticket report must not contain control characters".into());
            }
        }
        for summary in self.summaries.values().flatten() {
            check_text("Stage summary", summary, 2000)?;
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
) -> Result<UpdateRun, Box<dyn std::error::Error>> {
    let mut update = UpdateRun::new(
        serde_json::from_value(stage.into())?,
        serde_json::from_value(status.into())?,
    );
    let mut tickets: Vec<NewTicket> = Vec::new();
    let mut overrides: BTreeMap<Task, Role> = BTreeMap::new();
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
                    return Err(repeated().into());
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
                    return Err(repeated().into());
                }
                update.mode = if *flag == "--clear-mode" {
                    FieldUpdate::Clear
                } else {
                    let mode = next_value()?;
                    FieldUpdate::Set(serde_json::from_value(serde_json::Value::from(*mode)).map_err(|_| {
                        format!("Invalid value for --mode: {mode}; use factory-full, factory-balanced or factory-fast")
                    })?)
                };
                continue;
            }
            "--ticket" => {
                if !update.tickets.is_keep() && tickets.is_empty() {
                    return Err(repeated().into());
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
                    return Err(repeated().into());
                }
                update.tickets = FieldUpdate::Clear;
                continue;
            }
            "--model-routing" => {
                if !update.model_routing.is_keep() {
                    return Err(repeated().into());
                }
                let value = next_value()?;
                let (task, role) = split_pair(flag, value)?;
                let task: Task = serde_json::from_value(task.into()).map_err(|_| {
                    format!("Unknown task {task:?} for --model-routing; use build, check or finish")
                })?;
                let role: Role = serde_json::from_value(role.into()).map_err(|_| {
                    format!(
                        "Unknown role {role:?} for --model-routing; use strong, coding or routine"
                    )
                })?;
                if overrides.insert(task, role).is_some() {
                    return Err(format!("Role for {} given twice", task.name()).into());
                }
                continue;
            }
            "--clear-model-routing" => {
                if !update.model_routing.is_keep() || !overrides.is_empty() {
                    return Err(repeated().into());
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
            "--ticket-report" => {
                let value = next_value()?;
                let (reference, report) = split_pair(flag, value)?;
                let (kind, text) = report
                    .split_once(':')
                    .ok_or("Ticket report needs <reference>=<skill|activity>:<text>")?;
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
                    return Err(format!("Ticket status for {reference} given twice").into());
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
                    return Err(format!("Summary for {name} given twice").into());
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
            _ => return Err(format!("Unknown run update option: {flag}\n{UPDATE_HELP}").into()),
        };
        if !field.is_keep() {
            return Err(repeated().into());
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
    pub(crate) collect_usage: bool,
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
        let run = Run {
            id: id.to_string(),
            title: new.title,
            project: new.project,
            stage: Stage::Define,
            status: RunStatus::Running,
            activity: None,
            current_ticket: None,
            blocking_reason: None,
            rework: None,
            updated_at: Some(updated_now()?),
            repository: new.repository,
            worktree: None,
            branch: None,
            base_commit: None,
            mode: None,
            spec: None,
            pr: None,
            tickets: Vec::new(),
            ticket_reports: Vec::new(),
            summaries: BTreeMap::new(),
            timeline: Vec::new(),
            finished_at: None,
            sessions: new.session.into_iter().collect(),
            usage: None,
            pr_status: None,
            model_routing: new.model_routing,
        };
        let mut run = run;
        let created = run.updated_at.unwrap_or(0);
        mark(&mut run, created);
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
        next.stage = update.stage;
        next.status = update.status;
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
        if next.ticket_reports.len() + update.ticket_reports.len() > 1000 {
            return Err(io_invalid("A run can hold at most 1000 ticket reports"));
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
        for session in update.sessions {
            if !next.sessions.contains(&session) {
                next.sessions.push(session);
            }
        }
        if next.sessions.len() > KEEP_SESSIONS {
            next.sessions.drain(..next.sessions.len() - KEEP_SESSIONS);
        }
        next.updated_at = Some(updated_at);
        // Where the work is now, for attributing token usage; and when it ended.
        mark(&mut next, updated_at);
        next.finished_at = match (next.status, next.finished_at) {
            (RunStatus::Done, None) => Some(updated_at),
            (RunStatus::Done, kept) => kept,
            _ => None,
        };
        let previous = &runs[index];
        let collect_usage = previous.timeline.len() != next.timeline.len()
            || previous.sessions != next.sessions
            || (previous.finished_at.is_none() && next.finished_at.is_some());
        let old = std::mem::replace(&mut runs[index], next);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs[index] = old;
            return Err(error);
        }
        Ok(Some(UpdateOutcome {
            run: runs[index].clone(),
            collect_usage,
        }))
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

/// Appends a mark when the stage, rework or ticket in progress differs from the last one.
fn mark(run: &mut Run, at: u64) {
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
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn collection_decision_follows_persisted_transition() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = store
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        let apply = |stage, status, args: &[&str]| {
            store
                .update(&run.id, parse_update(stage, status, args).unwrap())
                .unwrap()
                .unwrap()
        };
        assert!(!apply("define", "running", &["--activity", "working"]).collect_usage);
        assert!(apply("build", "running", &[]).collect_usage);
        assert!(apply("build", "running", &["--session", "11111111-aaaa"]).collect_usage);
        assert!(!apply("build", "running", &["--session", "11111111-aaaa"]).collect_usage);
        assert!(apply("build", "done", &[]).collect_usage);
        assert!(!apply("build", "done", &[]).collect_usage);
        assert!(store
            .update("999999", UpdateRun::new(Stage::Build, RunStatus::Running))
            .unwrap()
            .is_none());
        let before = std::fs::read(&path).unwrap();
        let mut invalid = UpdateRun::new(Stage::Check, RunStatus::Running);
        invalid.branch = FieldUpdate::Set("invalid branch".into());
        assert!(store.update(&run.id, invalid).is_err());
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
        assert!(store
            .update(&run.id, UpdateRun::new(Stage::Check, RunStatus::Running))
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(serde_json::to_vec(&store.list().unwrap()).unwrap(), before);
    }

    #[test]
    fn invalid_mutations_leave_memory_and_disk_unchanged() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let make_new = |title: &str| NewRun {
            title: title.into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        };
        let run = store.create(make_new("valid")).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            store.create(make_new(" ")).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let mut update = UpdateRun::new(Stage::Build, RunStatus::Running);
        update.branch = FieldUpdate::Set("invalid branch".into());
        assert_eq!(
            store.update(&run.id, update).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(serde_json::to_vec(&store.list().unwrap()).unwrap(), before);
    }

    #[test]
    fn concurrent_creates_keep_all_records_and_unique_ids() {
        let dir = TestDir::new();
        let store = std::sync::Arc::new(RunStore::open(&dir.0.path().join("runs.json")).unwrap());
        let threads: Vec<_> = (0..16)
            .map(|n| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .create(NewRun {
                            title: format!("run {n}"),
                            project: "/work".into(),
                            repository: None,
                            session: None,
                            model_routing: None,
                        })
                        .unwrap()
                })
            })
            .collect();
        let created: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let ids: std::collections::HashSet<_> = created.iter().map(|run| run.id.clone()).collect();
        assert_eq!(ids.len(), 16);
        assert_eq!(
            load_runs(&dir.0.path().join("runs.json")).unwrap().len(),
            16
        );
    }

    #[test]
    fn concurrent_updates_to_different_runs_are_both_persisted() {
        let dir = TestDir::new();
        let store = std::sync::Arc::new(RunStore::open(&dir.0.path().join("runs.json")).unwrap());
        let first = store
            .create(NewRun {
                title: "first".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        let second = store
            .create(NewRun {
                title: "second".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        let updates = [(first.id, Stage::Build), (second.id, Stage::Check)];
        let threads: Vec<_> = updates
            .into_iter()
            .map(|(id, stage)| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .update(&id, UpdateRun::new(stage, RunStatus::Done))
                        .unwrap()
                        .unwrap()
                        .run
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let persisted = load_runs(&dir.0.path().join("runs.json")).unwrap();
        assert_eq!(persisted.len(), 2);
        assert!(persisted
            .iter()
            .all(|run| matches!(run.status, RunStatus::Done)));
        assert!(matches!(
            persisted
                .iter()
                .find(|run| run.title == "first")
                .unwrap()
                .stage,
            Stage::Build
        ));
        assert!(matches!(
            persisted
                .iter()
                .find(|run| run.title == "second")
                .unwrap()
                .stage,
            Stage::Check
        ));
    }

    #[test]
    fn cli_update_options_preserve_set_clear_and_reject_conflicts() {
        let update = parse_update("build", "running", &[]).unwrap();
        assert_eq!(
            serde_json::to_value(update).unwrap(),
            serde_json::json!({"stage":"build","status":"running"})
        );
        let update = parse_update(
            "build",
            "blocked",
            &[
                "--activity",
                "Implementing ticket",
                "--current-ticket",
                "https://example.com/42",
                "--blocking-reason",
                "Need input",
            ],
        )
        .unwrap();
        let value = serde_json::to_value(update).unwrap();
        assert_eq!(value["activity"], "Implementing ticket");
        assert_eq!(value["current_ticket"], "https://example.com/42");
        assert_eq!(value["blocking_reason"], "Need input");
        let update = parse_update(
            "finish",
            "done",
            &[
                "--clear-activity",
                "--clear-current-ticket",
                "--clear-blocking-reason",
            ],
        )
        .unwrap();
        let value = serde_json::to_value(update).unwrap();
        for field in ["activity", "current_ticket", "blocking_reason"] {
            assert_eq!(value.get(field), Some(&serde_json::Value::Null));
        }
        for options in [
            vec!["--activity"],
            vec!["--activity", "--clear-activity"],
            vec!["--clear-activity", "--activity", "x"],
            vec!["--clear-current-ticket", "--clear-current-ticket"],
            vec!["--unknown"],
            vec!["--current-ticket", "file:///tmp/ticket"],
            vec!["--blocking-reason", ""],
        ] {
            assert!(
                parse_update("build", "running", &options).is_err(),
                "options: {options:?}"
            );
        }
        assert!(parse_update("unknown", "running", &[]).is_err());
        assert!(parse_update("build", "unknown", &[]).is_err());
    }

    #[test]
    fn cli_rework_option_sets_clears_and_rejects_invalid_counts() {
        let update = parse_update("build", "running", &["--rework", "2"]).unwrap();
        assert_eq!(serde_json::to_value(update).unwrap()["rework"], 2);
        let update = parse_update("check", "running", &["--clear-rework"]).unwrap();
        assert_eq!(
            serde_json::to_value(update).unwrap().get("rework"),
            Some(&serde_json::Value::Null)
        );
        for options in [
            vec!["--rework"],
            vec!["--rework", "0"],
            vec!["--rework", "-1"],
            vec!["--rework", "two"],
            vec!["--rework", "1.5"],
            vec!["--rework", "100"],
            vec!["--rework", "1", "--clear-rework"],
            vec!["--rework", "1", "--rework", "2"],
        ] {
            assert!(
                parse_update("build", "running", &options).is_err(),
                "options: {options:?}"
            );
        }
        let range = format!("1 to {MAX_REWORK}");
        for count in ["0", "100", "two"] {
            let Err(error) = parse_update("build", "running", &["--rework", count]) else {
                panic!("--rework {count} should be rejected");
            };
            assert!(error.to_string().contains(&range), "{error}");
        }
        assert!(UPDATE_HELP.contains(&format!("--rework <1-{MAX_REWORK}>")));
    }

    #[test]
    fn failed_save_rolls_back_metadata_and_timestamp() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = store
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        let original = fs::read(&path).unwrap();
        fs::create_dir(path.with_extension("json.tmp")).unwrap();
        let update = parse_update(
            "build",
            "blocked",
            &["--activity", "Working", "--blocking-reason", "Need input"],
        )
        .unwrap();
        assert!(store.update(&run.id, update).is_err());
        assert_eq!(
            serde_json::to_vec(&store.list().unwrap()).unwrap(),
            original
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    fn new_run(store: &RunStore, title: &str) -> Run {
        store
            .create(NewRun {
                title: title.into(),
                project: "/work/repo.worktrees/feature".into(),
                repository: Some("/work/repo".into()),
                session: None,
                model_routing: None,
            })
            .unwrap()
    }

    fn selection() -> crate::routing::ModelRouting {
        serde_json::from_value(serde_json::json!({
            "agent": "claude",
            "roles": {
                "strong": {"model": "opus", "effort": "high"},
                "coding": {"model": "sonnet", "effort": "medium"}
            },
            "tasks": {"build": "coding", "check": "strong"}
        }))
        .unwrap()
    }

    fn routed_store(dir: &TestDir) -> (RunStore, Run) {
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = store
            .create(NewRun {
                title: "routed".into(),
                project: "/work/repo".into(),
                repository: None,
                session: None,
                model_routing: Some(selection()),
            })
            .unwrap();
        (store, run)
    }

    #[test]
    fn a_task_override_beats_the_policy_replaces_earlier_ones_clears_and_survives_a_restart() {
        use crate::routing::{Role, Task};
        let dir = TestDir::new();
        let (store, run) = routed_store(&dir);
        let routing =
            |store: &RunStore| store.get(&run.id).unwrap().unwrap().model_routing.unwrap();
        assert_eq!(routing(&store).role_for(Task::Check), Some(Role::Strong));

        store
            .update(&run.id, update(&["--model-routing", "check=coding"]))
            .unwrap();
        assert_eq!(routing(&store).role_for(Task::Check), Some(Role::Coding));
        assert_eq!(routing(&store).role_for(Task::Build), Some(Role::Coding));
        // The policy's own mapping is untouched.
        assert_eq!(routing(&store).tasks[&Task::Check], Role::Strong);

        // A new set of overrides replaces the old one, and an unrelated update keeps it.
        store
            .update(&run.id, update(&["--model-routing", "build=strong"]))
            .unwrap();
        assert_eq!(routing(&store).role_for(Task::Check), Some(Role::Strong));
        assert_eq!(routing(&store).role_for(Task::Build), Some(Role::Strong));
        store
            .update(&run.id, update(&["--activity", "later"]))
            .unwrap();
        assert_eq!(routing(&store).role_for(Task::Build), Some(Role::Strong));

        let reopened = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        assert_eq!(routing(&reopened).role_for(Task::Build), Some(Role::Strong));

        store
            .update(&run.id, update(&["--clear-model-routing"]))
            .unwrap();
        assert_eq!(routing(&store).role_for(Task::Build), Some(Role::Coding));
        assert!(routing(&store).overrides.is_empty());
    }

    #[test]
    fn bad_task_overrides_are_refused_and_change_nothing() {
        let dir = TestDir::new();
        let (store, run) = routed_store(&dir);
        // The command line refuses what cannot be a task or a role.
        for options in [
            &["--model-routing", "deploy=coding"][..],
            &["--model-routing", "check=boss"],
            &["--model-routing", "check"],
            &[
                "--model-routing",
                "check=coding",
                "--model-routing",
                "check=strong",
            ],
            &["--model-routing", "check=coding", "--clear-model-routing"],
        ] {
            assert!(
                parse_update("build", "running", options).is_err(),
                "{options:?}"
            );
        }
        // The run refuses a role it does not have, such as routine here.
        let error = store
            .update(&run.id, update(&["--model-routing", "finish=routine"]))
            .unwrap_err();
        assert!(error.to_string().contains("routine"), "{error}");
        assert!(store
            .get(&run.id)
            .unwrap()
            .unwrap()
            .model_routing
            .unwrap()
            .overrides
            .is_empty());
        // A run that has no selection has nothing to override.
        let plain = new_run(&store, "plain");
        let error = store
            .update(&plain.id, update(&["--model-routing", "check=coding"]))
            .unwrap_err();
        assert!(error.to_string().contains("model routing"), "{error}");
        // Clearing where there is nothing to clear is harmless.
        store
            .update(&plain.id, update(&["--clear-model-routing"]))
            .unwrap();
    }

    #[test]
    fn a_model_routing_selection_recorded_at_start_survives_updates_and_a_restart() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = store
            .create(NewRun {
                title: "routed".into(),
                project: "/work/repo".into(),
                repository: None,
                session: None,
                model_routing: Some(selection()),
            })
            .unwrap();
        assert_eq!(run.model_routing, Some(selection()));
        let run = store
            .update(&run.id, update(&["--activity", "later"]))
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(run.model_routing, Some(selection()));
        let reopened = RunStore::open(&path).unwrap();
        let run = reopened.get(&run.id).unwrap().unwrap();
        assert_eq!(run.model_routing, Some(selection()));
    }

    fn update(options: &[&str]) -> UpdateRun {
        parse_update("build", "running", options).unwrap()
    }

    #[test]
    fn context_is_set_kept_and_cleared_one_field_at_a_time() {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = new_run(&store, "task");
        assert_eq!(run.repository.as_deref(), Some("/work/repo"));
        assert!(run.worktree.is_none() && run.tickets.is_empty() && run.summaries.is_empty());

        let run = store
            .update(
                &run.id,
                update(&[
                    "--mode",
                    "factory-fast",
                    "--worktree",
                    "/work/repo.worktrees/feature",
                    "--branch",
                    "feature/x",
                    "--base-commit",
                    "ABCDEF1234",
                    "--spec",
                    "https://github.com/o/r/issues/1",
                    "--pr",
                    "https://github.com/o/r/pull/2",
                    "--summary",
                    "define=Spec written",
                ]),
            )
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(run.mode, Some(Mode::Fast));
        assert_eq!(run.branch.as_deref(), Some("feature/x"));
        assert_eq!(
            run.base_commit.as_deref(),
            Some("abcdef1234"),
            "stored in lowercase"
        );
        assert_eq!(run.summaries[&Stage::Define], "Spec written");

        // An update that mentions none of the context keeps all of it.
        let kept = store
            .update(&run.id, update(&["--activity", "Working"]))
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(kept.worktree, run.worktree);
        assert_eq!(kept.spec, run.spec);
        assert_eq!(kept.pr, run.pr);
        assert_eq!(kept.summaries, run.summaries);

        // Each clear flag removes only its own field.
        let cleared = store
            .update(
                &run.id,
                update(&["--clear-pr", "--clear-mode", "--clear-summary", "define"]),
            )
            .unwrap()
            .unwrap()
            .run;
        assert!(cleared.pr.is_none() && cleared.mode.is_none() && cleared.summaries.is_empty());
        assert_eq!(cleared.branch.as_deref(), Some("feature/x"));
        assert_eq!(cleared.spec, run.spec);
    }

    #[test]
    fn stage_summaries_survive_later_stages() {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = new_run(&store, "task");
        store
            .update(&run.id, update(&["--summary", "define=Planned"]))
            .unwrap();
        store
            .update(
                &run.id,
                parse_update(
                    "check",
                    "running",
                    &["--summary", "build=Built three tickets"],
                )
                .unwrap(),
            )
            .unwrap();
        let run = store
            .update(
                &run.id,
                parse_update("finish", "running", &["--summary", "define=Planned again"]).unwrap(),
            )
            .unwrap()
            .unwrap()
            .run;
        assert!(matches!(run.stage, Stage::Finish));
        assert_eq!(run.summaries[&Stage::Define], "Planned again");
        assert_eq!(run.summaries[&Stage::Build], "Built three tickets");
    }

    #[test]
    fn the_ticket_list_is_ordered_and_keeps_progress_when_relisted() {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = new_run(&store, "task");
        let run = store
            .update(
                &run.id,
                update(&[
                    "--ticket",
                    "https://x.test/1 First one",
                    "--ticket",
                    "docs/t2.md Second",
                    "--ticket-status",
                    "https://x.test/1=in-progress",
                ]),
            )
            .unwrap()
            .unwrap()
            .run;
        let view = |run: &Run| {
            run.tickets
                .iter()
                .map(|t| (t.reference.clone(), t.status))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            view(&run),
            [
                ("https://x.test/1".into(), TicketStatus::InProgress),
                ("docs/t2.md".into(), TicketStatus::Pending)
            ]
        );
        // Listing again in a new order with a new ticket keeps the status of the kept ones.
        let run = store
            .update(
                &run.id,
                update(&[
                    "--ticket",
                    "docs/t2.md Second",
                    "--ticket",
                    "https://x.test/1 First one",
                    "--ticket",
                    "docs/t3.md Third",
                ]),
            )
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(
            view(&run),
            [
                ("docs/t2.md".into(), TicketStatus::Pending),
                ("https://x.test/1".into(), TicketStatus::InProgress),
                ("docs/t3.md".into(), TicketStatus::Pending),
            ]
        );
        let run = store
            .update(
                &run.id,
                update(&[
                    "--ticket-status",
                    "https://x.test/1=done",
                    "--ticket-status",
                    "docs/t2.md=in-progress",
                ]),
            )
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(run.tickets[0].status, TicketStatus::InProgress);
        assert_eq!(run.tickets[1].status, TicketStatus::Done);
        let run = store
            .update(&run.id, update(&["--clear-tickets"]))
            .unwrap()
            .unwrap()
            .run;
        assert!(run.tickets.is_empty());
    }

    #[test]
    fn inconsistent_updates_are_rejected_with_a_reason_and_change_nothing() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = new_run(&store, "task");
        store
            .update(&run.id, update(&["--ticket", "a A", "--ticket", "b B"]))
            .unwrap();
        let before = fs::read(&path).unwrap();

        for (options, reason) in [
            (
                vec!["--ticket-status", "missing=done"],
                "No ticket missing is listed",
            ),
            (
                vec![
                    "--ticket-status",
                    "a=in-progress",
                    "--ticket-status",
                    "b=in-progress",
                ],
                "Only one ticket",
            ),
            // Even with a valid change beside it, nothing is applied.
            (
                vec!["--branch", "other", "--ticket-status", "zzz=done"],
                "No ticket zzz",
            ),
        ] {
            let error = store.update(&run.id, update(&options)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains(reason), "{error}");
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(store.get(&run.id).unwrap().unwrap().branch.is_none());
    }

    #[test]
    fn invalid_context_values_are_refused_with_a_message() {
        for (options, reason) in [
            (vec!["--mode", "turbo"], "factory-fast"),
            (vec!["--worktree", "relative/path"], "absolute path"),
            (vec!["--branch", "two words"], "branch name"),
            (vec!["--branch", "-x"], "branch name"),
            (vec!["--base-commit", "xyz"], "hexadecimal"),
            (
                vec!["--spec", "javascript:alert(1)"],
                "HTTP(S) URL or a local path",
            ),
            (
                vec!["--spec", "https://user:secret@example.com/spec"],
                "credentials",
            ),
            (vec!["--pr", "/local/path"], "HTTP(S) URL"),
            (
                vec!["--pr", "https://tok:x@example.com/pr/1"],
                "credentials",
            ),
            (vec!["--ticket", "only-a-reference"], "<reference> <title>"),
            (
                vec!["--ticket", "a A", "--ticket", "a Again"],
                "listed twice",
            ),
            (vec!["--ticket", "a A", "--clear-tickets"], "conflicting"),
            (vec!["--clear-tickets", "--ticket", "a A"], "conflicting"),
            (
                vec!["--ticket-status", "a=finished"],
                "pending, in-progress or done",
            ),
            (vec!["--ticket-status", "a"], "<reference>=<status>"),
            (
                vec!["--ticket-status", "a=done", "--ticket-status", "a=pending"],
                "twice",
            ),
            (
                vec!["--summary", "plan=x"],
                "define, build, check or finish",
            ),
            (vec!["--summary", "define"], "<name>=<value>"),
            (
                vec!["--summary", "define=a", "--clear-summary", "define"],
                "twice",
            ),
            (vec!["--mode", "factory", "--clear-mode"], "conflicting"),
            (
                vec!["--summary", "define=bad\u{1b}[2J"],
                "control characters",
            ),
        ] {
            let Err(error) = parse_update("build", "running", &options) else {
                panic!("{options:?} should be rejected");
            };
            assert!(error.to_string().contains(reason), "{options:?}: {error}");
        }
        // Windows and Unix absolute paths and local references are fine.
        for options in [
            vec!["--worktree", "C:\\work\\repo"],
            vec!["--worktree", "/work/repo"],
            vec!["--spec", "docs/specs/thing.md"],
            vec!["--spec", "C:\\specs\\thing.md"],
            vec!["--spec", "#12"],
        ] {
            assert!(
                parse_update("build", "running", &options).is_ok(),
                "{options:?}"
            );
        }
        let too_many: Vec<String> = (0..=MAX_TICKETS).map(|n| format!("t{n} Title")).collect();
        let options: Vec<&str> = too_many
            .iter()
            .flat_map(|t| ["--ticket", t.as_str()])
            .collect();
        assert!(parse_update("build", "running", &options)
            .unwrap_err()
            .to_string()
            .contains("At most 100"));
    }

    #[test]
    fn context_survives_a_restart_and_a_failed_save_changes_nothing() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = new_run(&store, "task");
        store
            .update(
                &run.id,
                update(&[
                    "--branch",
                    "feature/x",
                    "--ticket",
                    "a A",
                    "--summary",
                    "define=Planned",
                    "--pr",
                    "https://x.test/pr/1",
                ]),
            )
            .unwrap();
        let restarted = RunStore::open(&path).unwrap();
        let loaded = restarted.get(&run.id).unwrap().unwrap();
        assert_eq!(loaded.repository.as_deref(), Some("/work/repo"));
        assert_eq!(loaded.branch.as_deref(), Some("feature/x"));
        assert_eq!(loaded.tickets.len(), 1);
        assert_eq!(loaded.summaries[&Stage::Define], "Planned");

        let before = fs::read(&path).unwrap();
        fs::create_dir(path.with_extension("json.tmp")).unwrap();
        let result = restarted.update(
            &run.id,
            update(&[
                "--branch",
                "other",
                "--clear-tickets",
                "--clear-summary",
                "define",
                "--clear-pr",
            ]),
        );
        assert!(result.is_err());
        let after = restarted.get(&run.id).unwrap().unwrap();
        assert_eq!(after.branch.as_deref(), Some("feature/x"));
        assert_eq!(after.tickets.len(), 1);
        assert_eq!(after.summaries.len(), 1);
        assert!(after.pr.is_some());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn legacy_full_mode_loads_without_rewriting_and_normalizes_on_update() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let legacy = r#"[{"id":"7","title":"old","project":"/work/old","stage":"build","status":"running","updated_at":5,"mode":"factory","branch":"feature/x","base_commit":"abc1234","spec":"docs/spec.md"}]"#;
        fs::write(&path, legacy).unwrap();
        let store = RunStore::open(&path).unwrap();
        let run = store.get("7").unwrap().unwrap();
        assert_eq!(run.mode, Some(Mode::Full));
        assert_eq!(run.branch.as_deref(), Some("feature/x"));
        assert_eq!(run.spec.as_deref(), Some("docs/spec.md"));
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
        store
            .update("7", update(&["--activity", "Continuing"]))
            .unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored[0]["mode"], "factory-full");
        assert_eq!(stored[0]["id"], "7");
        assert_eq!(stored[0]["stage"], "build");
        assert_eq!(stored[0]["branch"], "feature/x");
    }

    #[test]
    fn records_from_before_run_context_load_with_everything_unknown() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let legacy = r#"[{"id":"7","title":"old","project":"/work/old","stage":"build","status":"blocked","blocking_reason":"Need input","updated_at":5}]"#;
        fs::write(&path, legacy).unwrap();
        let store = RunStore::open(&path).unwrap();
        let run = store.get("7").unwrap().unwrap();
        assert_eq!(run.blocking_reason.as_deref(), Some("Need input"));
        assert!(run.repository.is_none() && run.worktree.is_none() && run.branch.is_none());
        assert!(
            run.base_commit.is_none()
                && run.mode.is_none()
                && run.spec.is_none()
                && run.pr.is_none()
        );
        assert!(run.tickets.is_empty() && run.summaries.is_empty());
        assert!(run.model_routing.is_none());
        // Updating it adds context without inventing the rest, and keeps what it had.
        let run = store
            .update("7", update(&["--branch", "b"]))
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(run.blocking_reason.as_deref(), Some("Need input"));
        assert!(run.repository.is_none() && run.worktree.is_none());
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(json["repository"], serde_json::Value::Null);
        assert_eq!(json["tickets"], serde_json::json!([]));
        assert!(json.get("model_routing").is_none());
    }

    #[test]
    fn concurrent_runs_keep_independent_context() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = std::sync::Arc::new(RunStore::open(&path).unwrap());
        let runs: Vec<Run> = (0..8)
            .map(|n| new_run(&store, &format!("run {n}")))
            .collect();
        let threads: Vec<_> = runs
            .iter()
            .enumerate()
            .map(|(n, run)| {
                let (store, id) = (store.clone(), run.id.clone());
                std::thread::spawn(move || {
                    let branch = format!("feature/{n}");
                    let ticket = format!("t{n} Ticket {n}");
                    let summary = format!("define=Summary {n}");
                    store
                        .update(
                            &id,
                            parse_update(
                                "build",
                                "running",
                                &[
                                    "--branch",
                                    &branch,
                                    "--ticket",
                                    &ticket,
                                    "--summary",
                                    &summary,
                                ],
                            )
                            .unwrap(),
                        )
                        .unwrap()
                        .unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let persisted = load_runs(&path).unwrap();
        assert_eq!(persisted.len(), 8);
        for (n, run) in runs.iter().enumerate() {
            let stored = persisted.iter().find(|r| r.id == run.id).unwrap();
            assert_eq!(
                stored.branch.as_deref(),
                Some(format!("feature/{n}").as_str())
            );
            assert_eq!(stored.tickets.len(), 1);
            assert_eq!(stored.tickets[0].reference, format!("t{n}"));
            assert_eq!(stored.summaries[&Stage::Define], format!("Summary {n}"));
        }
    }

    #[test]
    fn urls_and_names_cannot_smuggle_credentials_or_tokens() {
        for (options, reason) in [
            (vec!["--spec", " https://tok:secret@evil.test/x"], "spaces"),
            (vec!["--spec", "https://evil.test/x "], "spaces"),
            (
                vec!["--spec", "HTTPS://tok:secret@evil.test/x"],
                "credentials",
            ),
            (
                vec!["--spec", "https://example.com/spec?token=abc"],
                "query string",
            ),
            (
                vec!["--spec", "https://example.com/spec#frag"],
                "query string",
            ),
            (
                vec!["--pr", "https://example.com/pr/1?access_token=abc"],
                "query string",
            ),
            (
                vec!["--ticket", "https://example.com/t?key=1 Title"],
                "query string",
            ),
            (vec!["--branch", "a:b"], "branch name"),
            (vec!["--branch", "x~1"], "branch name"),
            (vec!["--branch", "a@{b}"], "branch name"),
            (vec!["--branch", "topic.lock"], "branch name"),
            (vec!["--branch", "trailing/"], "branch name"),
            (vec!["--branch", "back\\slash"], "branch name"),
        ] {
            let Err(error) = parse_update("build", "running", &options) else {
                panic!("{options:?} should be rejected");
            };
            assert!(error.to_string().contains(reason), "{options:?}: {error}");
        }
        for options in [
            vec!["--branch", "feature/run-context_2"],
            vec!["--spec", "https://github.com/o/r/issues/1"],
        ] {
            assert!(
                parse_update("build", "running", &options).is_ok(),
                "{options:?}"
            );
        }
    }

    #[test]
    fn simultaneous_updates_to_one_run_both_land() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = std::sync::Arc::new(RunStore::open(&path).unwrap());
        let run = new_run(&store, "task");
        let threads: Vec<_> = [
            vec!["--branch", "feature/a"],
            vec!["--summary", "build=Built"],
            vec!["--pr", "https://x.test/pr/1"],
            vec!["--ticket", "t T"],
        ]
        .into_iter()
        .map(|options| {
            let (store, id) = (store.clone(), run.id.clone());
            std::thread::spawn(move || {
                let options: Vec<&str> = options.clone();
                store
                    .update(&id, parse_update("build", "running", &options).unwrap())
                    .unwrap()
                    .unwrap();
            })
        })
        .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        // Each update changed only its own fields, so none overwrote another.
        let stored = load_runs(&path).unwrap().pop().unwrap();
        assert_eq!(stored.branch.as_deref(), Some("feature/a"));
        assert_eq!(stored.summaries.len(), 1);
        assert_eq!(stored.pr.as_deref(), Some("https://x.test/pr/1"));
        assert_eq!(stored.tickets.len(), 1);
    }

    #[test]
    fn absolute_paths_are_recognised_the_same_way_on_every_system() {
        for yes in [
            "/work/repo",
            "C:\\work\\repo",
            "c:/work/repo",
            "\\\\server\\share\\repo",
        ] {
            assert!(is_absolute_path(yes), "{yes}");
        }
        for no in [
            "work/repo",
            "./repo",
            "..\\repo",
            "C:repo",
            "C:",
            "",
            "~/repo",
        ] {
            assert!(!is_absolute_path(no), "{no}");
        }
    }

    #[test]
    fn the_timeline_records_where_the_work_is_for_attributing_usage() {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = new_run(&store, "task");
        assert_eq!(run.timeline.len(), 1, "a run starts in Define");
        assert!(matches!(run.timeline[0].stage, Stage::Define) && run.finished_at.is_none());

        let after = |options: &[&str], stage: &str| {
            store
                .update(&run.id, parse_update(stage, "running", options).unwrap())
                .unwrap()
                .unwrap()
                .run
        };
        // An update that changes nothing about the stage, ticket or rework adds no mark.
        assert_eq!(
            after(&["--activity", "thinking"], "define").timeline.len(),
            1
        );
        after(&["--ticket", "t1 One", "--ticket", "t2 Two"], "build");
        let started = after(&["--ticket-status", "t1=in-progress"], "build");
        assert_eq!(started.timeline.len(), 3);
        assert_eq!(started.timeline[2].ticket.as_deref(), Some("t1"));
        let second = after(
            &[
                "--ticket-status",
                "t1=done",
                "--ticket-status",
                "t2=in-progress",
            ],
            "build",
        );
        assert_eq!(second.timeline[3].ticket.as_deref(), Some("t2"));
        let checking = after(&[], "check");
        assert_eq!(checking.timeline[4].ticket, None, "only Build has a ticket");
        let rework = after(&["--rework", "1"], "build");
        assert_eq!(rework.timeline.last().unwrap().rework, Some(1));
        assert!(rework
            .timeline
            .windows(2)
            .all(|pair| pair[0].at <= pair[1].at));
    }

    #[test]
    fn finishing_a_run_closes_its_window_and_reopening_clears_it() {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = new_run(&store, "task");
        let done = store
            .update(&run.id, parse_update("finish", "done", &[]).unwrap())
            .unwrap()
            .unwrap()
            .run;
        let end = done.finished_at.expect("closed when done");
        let again = store
            .update(
                &run.id,
                parse_update("finish", "done", &["--activity", "x"]).unwrap(),
            )
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(
            again.finished_at,
            Some(end),
            "a second done update keeps the first end"
        );
        let reopened = store
            .update(&run.id, parse_update("build", "running", &[]).unwrap())
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(reopened.finished_at, None);
    }

    #[test]
    fn ticket_reports_reject_bad_values_and_unknown_tickets_without_saving() {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let created = new_run(&store, "task");
        for value in ["missing", "t1=bad:x", "t1=skill:", "t1=activity:bad\ntext"] {
            assert!(
                parse_update("build", "running", &["--ticket-report", value]).is_err(),
                "{value:?}"
            );
        }
        let update =
            parse_update("build", "running", &["--ticket-report", "t1=skill:tdd"]).unwrap();
        assert!(store.update(&created.id, update).is_err());
        assert!(store
            .get(&created.id)
            .unwrap()
            .unwrap()
            .ticket_reports
            .is_empty());
    }

    #[test]
    fn ticket_reports_survive_updates_and_reopening_the_store() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let created = new_run(&store, "task");
        store
            .update(
                &created.id,
                parse_update(
                    "build",
                    "running",
                    &[
                        "--ticket",
                        "t1 First ticket",
                        "--ticket-report",
                        "t1=skill:tdd",
                    ],
                )
                .unwrap(),
            )
            .unwrap()
            .unwrap();
        store
            .update(
                &created.id,
                parse_update(
                    "build",
                    "running",
                    &["--ticket-report", "t1=activity:Tests passed"],
                )
                .unwrap(),
            )
            .unwrap()
            .unwrap();
        let reopened = RunStore::open(&path).unwrap();
        let saved = reopened.get(&created.id).unwrap().unwrap();
        let json = serde_json::to_value(saved).unwrap();
        assert_eq!(json["ticket_reports"][0]["ticket"], "t1");
        assert_eq!(json["ticket_reports"][0]["kind"], "skill");
        assert_eq!(json["ticket_reports"][0]["text"], "tdd");
        assert_eq!(json["ticket_reports"][1]["text"], "Tests passed");
        assert_eq!(json["ticket_reports"][1]["stage"], "build");
        assert!(json["ticket_reports"][1]["at"].as_u64().is_some());
    }

    #[test]
    fn sessions_are_recorded_once_validated_and_kept_across_updates() {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let created = store
            .create(NewRun {
                title: "t".into(),
                project: "/p".into(),
                repository: None,
                session: Some("11111111-aaaa".into()),
                model_routing: None,
            })
            .unwrap();
        assert_eq!(created.sessions, ["11111111-aaaa"]);
        let run = store
            .update(
                &created.id,
                parse_update(
                    "build",
                    "running",
                    &[
                        "--session",
                        "22222222-bbbb",
                        "--session",
                        "11111111-aaaa",
                        "--session",
                        "22222222-bbbb",
                    ],
                )
                .unwrap(),
            )
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(run.sessions, ["11111111-aaaa", "22222222-bbbb"]);
        let kept = store
            .update(&created.id, update(&["--activity", "x"]))
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(kept.sessions, run.sessions);
        for bad in [
            "short",
            "has space in it",
            "../../etc/passwd",
            "semi;colon-id",
            "",
        ] {
            assert!(
                parse_update("build", "running", &["--session", bad]).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn usage_is_replaced_atomically_and_a_failed_save_keeps_the_old_usage() {
        use crate::usage::{Usage, UsageStatus};
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = new_run(&store, "task");
        assert!(run.usage.is_none());
        store
            .set_usage(&run.id, Usage::not_tracked("first", 1))
            .unwrap();
        fs::create_dir(path.with_extension("json.tmp")).unwrap();
        assert!(store
            .set_usage(&run.id, Usage::not_tracked("second", 2))
            .is_err());
        assert_eq!(
            store
                .get(&run.id)
                .unwrap()
                .unwrap()
                .usage
                .unwrap()
                .reason
                .as_deref(),
            Some("first")
        );
        fs::remove_dir(path.with_extension("json.tmp")).unwrap();
        store
            .set_usage(&run.id, Usage::not_tracked("third", 3))
            .unwrap();
        let restarted = RunStore::open(&path).unwrap();
        let usage = restarted.get(&run.id).unwrap().unwrap().usage.unwrap();
        assert_eq!(
            (usage.status, usage.reason.as_deref()),
            (UsageStatus::NotTracked, Some("third"))
        );
        // An unknown run is left alone.
        store.set_usage("nope", Usage::not_tracked("x", 1)).unwrap();
    }

    #[test]
    fn records_from_before_usage_tracking_load_without_a_timeline_or_usage() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        fs::write(&path, r#"[{"id":"7","title":"old","project":"/p","stage":"build","status":"running","branch":"b"}]"#).unwrap();
        let store = RunStore::open(&path).unwrap();
        let run = store.get("7").unwrap().unwrap();
        assert!(
            run.timeline.is_empty()
                && run.sessions.is_empty()
                && run.usage.is_none()
                && run.finished_at.is_none()
        );
        // Updating it starts a timeline from now; nothing earlier is invented.
        let run = store
            .update("7", update(&["--activity", "x"]))
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(run.timeline.len(), 1);
        assert_eq!(run.timeline[0].at, run.updated_at.unwrap());
    }
}
