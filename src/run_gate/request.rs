//! Requests and their options: what `jidoka run gate` and `jidoka run advance` send to the dashboard
//! server, and the server-side handling of an advance request.

use crate::{
    git::Git,
    pr_host::PrHost,
    run::{
        AdvanceMetadata, Exit, MAX_NOTE_BYTES, RunStore, SummariesUpdate, check_text,
        validate_agent_homes, validate_sessions,
    },
    stages::Gate,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A gate result as an agent reports it with `jidoka run gate`.
#[derive(Deserialize, Serialize)]
pub(crate) struct Record {
    pub(crate) gate: Gate,
    pub(crate) passed: bool,
    pub(crate) evidence: Option<String>,
    #[serde(default)]
    pub(crate) check: Option<String>,
}

pub(crate) fn parse_options(options: &[&str]) -> Result<(Option<String>, Option<String>), String> {
    let mut evidence = None;
    let mut check = None;
    let mut remaining = options;
    while !remaining.is_empty() {
        match remaining {
            ["--evidence", text, rest @ ..] if evidence.is_none() => {
                evidence = Some((*text).to_string());
                remaining = rest;
            }
            ["--check", name, rest @ ..] if check.is_none() => {
                check = Some((*name).to_string());
                remaining = rest;
            }
            _ => return Err("Use --check <name> and/or --evidence <text>".into()),
        }
    }
    Ok((check, evidence))
}

/// The requested stage transition and metadata to save with it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdvanceRequest {
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<String>,
    #[serde(flatten)]
    pub(super) summaries: SummariesUpdate,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sessions: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) agent_homes: BTreeMap<String, String>,
}

impl AdvanceRequest {
    fn validate(&self) -> Result<(), String> {
        self.forced_reason()?;
        if let Some(activity) = &self.activity {
            check_text("Activity", activity, MAX_NOTE_BYTES)?;
        }
        self.summaries.validate()?;
        validate_sessions(&self.sessions)?;
        validate_agent_homes(&self.agent_homes)
    }

    /// Advances run `id` in `store`, forced or once the exit gates pass.
    ///
    /// The request's metadata is saved with the Stage exit; leaving Finish also needs passing CI.
    ///
    /// # Errors
    ///
    /// Returns a message when the request is invalid, a gate or CI has not passed, the run is missing or
    /// changed while checking, or the run cannot be saved.
    pub(crate) fn advance(
        self,
        store: &RunStore,
        id: &str,
        git: &dyn Git,
        host: &dyn PrHost,
    ) -> Result<(), String> {
        self.validate()?;
        let forced = self.forced_reason()?.map(str::to_owned);
        let metadata = AdvanceMetadata {
            activity: self.activity,
            summaries: self.summaries,
            sessions: self.sessions,
            agent_homes: self.agent_homes,
        };
        store.advance(id, metadata, |run| match forced {
            Some(reason) => Ok(Exit::Override { reason }),
            None => super::accepted_exit(run, git, host),
        })
    }

    /// The reason for a forced advance, or `None` for a normal one.
    ///
    /// # Errors
    ///
    /// Returns a message when `force` and a nonempty `reason` do not come together.
    pub(crate) fn forced_reason(&self) -> Result<Option<&str>, String> {
        let reason = self.reason.as_deref().map(str::trim);
        match (self.force, reason) {
            (true, Some(reason)) if !reason.is_empty() => Ok(Some(reason)),
            (true, _) => Err("--force needs a nonempty --reason \"<text>\"".into()),
            (false, None) => Ok(None),
            (false, Some(_)) => Err("--reason is only for --force".into()),
        }
    }
}

/// What `jidoka run advance <id> [options]` was asked to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AdvanceOptions {
    pub(crate) request: AdvanceRequest,
    pub(crate) json: bool,
}

/// Reads the options of `run advance`: `--summary <stage>=<text>`, `--activity <text>`,
/// `--json` and `--force --reason <text>`, in any order and each at most once.
///
/// # Errors
///
/// Returns a message for an unknown or repeated option, a missing value, or `--force` without a reason.
pub(crate) fn parse_advance_options(options: &[&str]) -> Result<AdvanceOptions, String> {
    let mut parsed = AdvanceOptions::default();
    let mut seen: Vec<&str> = Vec::new();
    let mut rest = options.iter().copied();
    while let Some(option) = rest.next() {
        if seen.contains(&option) {
            return Err(format!("{option} is given more than once"));
        }
        let mut value = || {
            rest.next()
                .filter(|value| !value.starts_with("--") || value.contains(char::is_whitespace))
                .ok_or_else(|| format!("{option} needs a value"))
        };
        match option {
            "--summary" => {
                let summary = value()?;
                parsed
                    .request
                    .summaries
                    .parse_option(option, &mut || Ok(&summary))?;
            }
            "--activity" => parsed.request.activity = Some(value()?.to_owned()),
            "--reason" => parsed.request.reason = Some(value()?.to_owned()),
            "--force" => parsed.request.force = true,
            "--json" => parsed.json = true,
            _ => return Err(format!("Unknown option {option}")),
        }
        seen.push(option);
    }
    parsed.request.validate()?;
    Ok(parsed)
}

/// `jidoka run gate <id> <gate> <pass|fail> [--check <name>] [--evidence <text>]`: records a gate result.
pub(crate) struct GateCommand;

impl crate::cli::Command for GateCommand {
    /// The run id and the gate result to record.
    type Options = (String, Record);
    type Outcome = ();

    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn std::error::Error>> {
        let [id, gate, result, options @ ..] = words else {
            return Err(crate::cli::usage_error("Invalid arguments.").into());
        };
        crate::client::check_run_id(id)?;
        let gate = serde_json::from_value(serde_json::Value::String((*gate).to_string()))
            .map_err(|_| "Unknown gate")?;
        let passed = match *result {
            "pass" => true,
            "fail" => false,
            _ => return Err("Use pass or fail".into()),
        };
        let (check, evidence) = parse_options(options)?;
        let record = Record {
            gate,
            passed,
            evidence,
            check,
        };
        Ok(((*id).to_owned(), record))
    }

    fn run((id, record): &Self::Options) -> Result<(), Box<dyn std::error::Error>> {
        crate::client::post(
            &format!("/api/runs/{id}/gate"),
            &serde_json::to_vec(record)?,
        )?;
        Ok(())
    }

    fn render(
        (id, _): &Self::Options,
        (): (),
    ) -> Result<crate::cli::Output, Box<dyn std::error::Error>> {
        Ok(crate::cli::Output::text(format!(
            "Gate recorded for run {id}\n"
        )))
    }
}

/// `jidoka run advance <id> [options]`: leaves the current Stage, then shows the new one as `run next` does.
pub(crate) struct AdvanceCommand;

impl crate::cli::Command for AdvanceCommand {
    type Options = (String, AdvanceOptions);
    /// The new stage. Reading it can fail after the run moved on, which the output still reports.
    type Outcome = Result<crate::run_next::Next, Box<dyn std::error::Error>>;

    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn std::error::Error>> {
        let (id, options) = crate::cli::split_first(words);
        crate::client::check_run_id(id)?;
        let parsed = parse_advance_options(options).map_err(crate::cli::usage_error)?;
        Ok((id.to_owned(), parsed))
    }

    fn run((id, parsed): &Self::Options) -> Result<Self::Outcome, Box<dyn std::error::Error>> {
        let mut request = parsed.request.clone();
        let sessions = crate::run_begin::current_agent_sessions();
        request.sessions = sessions.iter().map(|session| session.id.clone()).collect();
        request.agent_homes = crate::run_begin::agent_homes(&sessions);
        crate::client::post(
            &format!("/api/runs/{id}/advance"),
            &serde_json::to_vec(&request)?,
        )?;
        Ok(crate::run_next::next_for(id))
    }

    fn render(
        (id, parsed): &Self::Options,
        next: Self::Outcome,
    ) -> Result<crate::cli::Output, Box<dyn std::error::Error>> {
        if parsed.json {
            return Ok(crate::cli::Output::json(&next?)?);
        }
        let advanced = crate::cli::Output::text(format!("Advanced run {id}\n\n"));
        Ok(match next {
            Ok(next) => crate::cli::Output::text(advanced.stdout + &crate::run_next::render(&next)),
            Err(error) => advanced.failed(error),
        })
    }
}
