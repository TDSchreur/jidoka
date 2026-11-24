//! Gate evaluation, gate recording and the live CI check that decide how a run may leave a Stage.
use crate::{
    checks::{self, Selection},
    git::Git,
    pr_host::PrHost,
    pr_state, pr_wait,
    run::{
        AdvanceMetadata, CiEvidence, CiOutcome, Evaluation, Exit, GateResult, GateStatus,
        MAX_GATE_RESULTS, MAX_NOTE_BYTES, Mode, Run, RunStatus, RunStore, SkippedCheck, Stage,
        SummariesUpdate, Ticket, TicketStatus, check_text, updated_now, validate_agent_homes,
        validate_sessions,
    },
    stages::{self, Destination, Gate, Transition},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

/// Allow the existing 90-second CI discovery grace period, including a final poll.
const CI_TIMEOUT: Duration = Duration::from_secs(95);

#[derive(Deserialize, Serialize)]
pub(crate) struct Record {
    pub(crate) gate: Gate,
    pub(crate) passed: bool,
    pub(crate) evidence: Option<String>,
    #[serde(default)]
    pub(crate) check: Option<String>,
}

const RESOLVE_FINDINGS: &str = "resolve them with `jidoka run update <id> --finding-status <finding-id>=<fixed|dismissed|deferred>[:<reason>]` or start it with `jidoka run rework`";
/// The remedy `advance` names for this gate, which fails either on open Findings or on no recorded pass.
const RECORD_OR_RESOLVE_FINDINGS: &str = "record `jidoka run gate <id> no-actionable-findings pass` once no Finding is open; resolve open Findings with `jidoka run update <id> --finding-status <finding-id>=<fixed|dismissed|deferred>[:<reason>]` or start it with `jidoka run rework`";

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

/// Refuses while a Finding is open, naming the ids and how to resolve them.
fn no_open_findings(run: &Run) -> Result<(), String> {
    let open: Vec<&str> = run
        .open_findings()
        .map(|finding| finding.id.as_str())
        .collect();
    if open.is_empty() {
        return Ok(());
    }
    Err(format!("Findings {} are open", open.join(", ")))
}

/// Checks `record` against the run and returns the gate result to store.
///
/// # Errors
///
/// Returns a message when the run is not running, the gate is checked automatically, or the evidence,
/// check or worktree does not support the result.
pub(crate) fn record(run: &Run, record: Record, git: &dyn Git) -> Result<GateResult, String> {
    if run.status != RunStatus::Running {
        return Err("Resume the run before recording a gate".into());
    }
    if record
        .evidence
        .as_ref()
        .is_some_and(|s| s.trim().is_empty() || s.len() > 2000)
    {
        return Err("Evidence must be non-empty and at most 2000 bytes".into());
    }
    if record.gate == Gate::NoActionableFindings && record.passed {
        no_open_findings(run).map_err(|problem| format!("{problem}; {RESOLVE_FINDINGS}"))?;
    }
    if matches!(
        record.gate,
        Gate::TicketsDone | Gate::FinishSummary | Gate::CiPassed
    ) {
        return Err("This gate is checked automatically".into());
    }
    if run.gates.len() >= MAX_GATE_RESULTS {
        return Err(format!(
            "A run can hold at most {MAX_GATE_RESULTS} gate results"
        ));
    }
    let check_head_before = if record.gate == Gate::ChecksRecorded {
        Some(run.worktree_head(git)?)
    } else {
        None
    };
    let skipped_checks = if record.gate == Gate::ChecksRecorded {
        if run.stage != Stage::Check {
            return Err("Record check results in the Check stage".into());
        }
        let mode = run.mode.ok_or("Record the run's mode first")?;
        let selection = select_checks(run, mode, git)?;
        let name = record
            .check
            .as_deref()
            .ok_or("Use --check <name> to identify the completed check")?;
        if !selection
            .applicable
            .iter()
            .any(|entry| entry.check.name == name && entry.check.gate.gate() == record.gate)
        {
            return Err(format!(
                "Check {name:?} is not applicable to this gate and diff"
            ));
        }
        skipped_snapshot(&selection)
    } else {
        if record.check.is_some() {
            return Err("--check is only valid for checks-recorded".into());
        }
        Vec::new()
    };
    let (ticket, work_attempt) = if record.gate == Gate::TicketsVerified {
        let (ticket, attempt) = ticket_verification_target(run)?;
        (ticket.map(|t| t.reference.clone()), Some(attempt))
    } else {
        (None, None)
    };
    let commit = if matches!(
        record.gate,
        Gate::Verified | Gate::ChecksRecorded | Gate::TicketsVerified
    ) && record.passed
    {
        Some(run.worktree_head(git)?)
    } else {
        None
    };
    if let Some(before) = check_head_before
        && run.worktree_head(git)? != before
    {
        return Err("The worktree HEAD changed while selecting checks; try again".into());
    }
    Ok(GateResult {
        gate: record.gate,
        stage: run.stage,
        rework: run.rework,
        passed: record.passed,
        evidence: record.evidence,
        commit,
        ticket,
        work_attempt,
        check: record.check,
        skipped_checks,
    })
}

fn ticket_verification_target(run: &Run) -> Result<(Option<&Ticket>, u64), String> {
    if run.stage != Stage::Build || run.mode != Some(Mode::Full) {
        return Err("Record tickets-verified in Full Build".into());
    }
    if let Some(ticket) = run.active_ticket() {
        return Ok((Some(ticket), ticket.work_attempt));
    }
    if let Some(attempt) = run.unticketed_attempt {
        return Ok((None, attempt));
    }
    Err("Mark a listed ticket in progress before recording its verification, or declare unticketed work".into())
}

fn ticket_plan_recorded(run: &Run) -> Result<(), String> {
    require(
        !run.tickets.is_empty() || run.unticketed_attempt.is_some(),
        "Build has no ticket plan",
    )
}

fn tickets_done(run: &Run) -> Result<(), String> {
    ticket_plan_recorded(run)?;
    require(
        run.tickets
            .iter()
            .all(|ticket| ticket.status == TicketStatus::Done),
        "Some tickets are not done",
    )
}

pub(crate) fn latest_ticket_verification<'a>(
    run: &'a Run,
    ticket: &Ticket,
) -> Option<&'a GateResult> {
    run.gates.iter().rev().find(|result| {
        result.gate == Gate::TicketsVerified
            && result.stage == Stage::Build
            && result.ticket.as_deref() == Some(&ticket.reference)
            && result.work_attempt == Some(ticket.work_attempt)
    })
}

fn unticketed_verification(run: &Run, attempt: u64) -> Option<&GateResult> {
    run.gates.iter().rev().find(|result| {
        result.gate == Gate::TicketsVerified
            && result.stage == Stage::Build
            && result.rework == run.rework
            && result.ticket.is_none()
            && result.work_attempt == Some(attempt)
    })
}

fn tickets_verified<'a>(
    run: &'a Run,
    git: &dyn Git,
    head: &str,
) -> Result<Vec<&'a GateResult>, String> {
    ticket_plan_recorded(run)?;
    if let Some(attempt) = run.unticketed_attempt {
        return unticketed_verification(run, attempt)
            .filter(|result| result.passed && result.commit.as_deref() == Some(head))
            .map(|result| vec![result])
            .ok_or_else(|| {
                "Unticketed Build has no passing verification on its current HEAD".into()
            });
    }
    let worktree = run
        .worktree
        .as_deref()
        .ok_or("Record the run's worktree first")?;
    let mut passed = Vec::new();
    for ticket in &run.tickets {
        let (result, commit) = latest_ticket_verification(run, ticket)
            .filter(|result| result.passed)
            .and_then(|result| Some((result, result.commit.as_deref()?)))
            .ok_or_else(|| {
                format!(
                    "Ticket {} has no passing verification for its current attempt",
                    ticket.reference
                )
            })?;
        if commit != head {
            match git.status(
                Path::new(worktree),
                &["merge-base", "--is-ancestor", commit, head],
            ) {
                Ok(0) => {}
                Ok(1) => {
                    return Err(format!(
                        "Ticket {} was verified on {commit}, which is outside the current Git history",
                        ticket.reference
                    ));
                }
                Ok(code) => {
                    return Err(format!(
                        "Could not check the verified commit {commit} for ticket {}: git exited with {code}",
                        ticket.reference
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "Could not check the verified commit {commit} for ticket {}: {error}",
                        ticket.reference
                    ));
                }
            }
        }
        passed.push(result);
    }
    Ok(passed)
}

fn skipped_snapshot(selection: &Selection) -> Vec<SkippedCheck> {
    selection
        .skipped
        .iter()
        .map(|entry| SkippedCheck {
            name: entry.check.name.to_owned(),
            reason: entry.reason.clone(),
        })
        .collect()
}

fn latest(run: &Run, gate: Gate) -> Option<&GateResult> {
    run.gates
        .iter()
        .rev()
        .find(|r| r.gate == gate && r.stage == run.stage && r.rework == run.rework)
}

fn latest_check<'a>(run: &'a Run, gate: Gate, check: &str) -> Option<&'a GateResult> {
    run.gates.iter().rev().find(|r| {
        r.gate == gate
            && r.stage == run.stage
            && r.rework == run.rework
            && r.check.as_deref() == Some(check)
    })
}

fn check_results<'a>(
    run: &'a Run,
    selection: &Selection,
    gate: Gate,
    head: &str,
) -> Result<Vec<&'a GateResult>, String> {
    selection
        .applicable
        .iter()
        .filter(|entry| entry.check.gate.gate() == gate)
        .map(|entry| {
            latest_check(run, gate, entry.check.name)
                .filter(|result| result.passed && result.commit.as_deref() == Some(head))
                .ok_or_else(|| format!("Check {} has no passing result on HEAD", entry.check.name))
        })
        .collect()
}

/// Determines when to assess a blocked run and whether to preview checks outside Check.
#[derive(Clone, Copy)]
pub(crate) enum AssessmentPurpose {
    Advance,
    Guidance,
    Dashboard,
}

pub(crate) enum CheckSelection {
    NotApplicable,
    Unavailable(String),
    Selected { mode: Mode, selection: Selection },
}

pub(crate) struct LocalAssessment {
    pub(crate) transition: Result<Transition, String>,
    pub(crate) evaluations: Vec<Evaluation>,
    pub(crate) checks: CheckSelection,
    candidate: Option<Result<String, String>>,
}

/// Collects and evaluates local Stage exit evidence. CI requires a separate live check.
pub(crate) fn assess_local(
    run: &Run,
    git: &dyn Git,
    purpose: AssessmentPurpose,
) -> LocalAssessment {
    let status = match (purpose, run.status) {
        (AssessmentPurpose::Dashboard, RunStatus::Blocked) => RunStatus::Running,
        _ => run.status,
    };
    let transition = run
        .mode
        .ok_or_else(|| "Record the run's mode first".to_string())
        .and_then(|mode| stages::next(mode, run.stage, status));
    let gates = transition.as_ref().map_or(&[][..], |next| next.gates);
    let candidate = candidate(run, gates, git);
    let checks = select_for_assessment(run, git, purpose);
    let evaluations = evaluate(run, gates, git, &candidate, &checks);
    LocalAssessment {
        transition,
        evaluations,
        checks,
        candidate,
    }
}

fn select_for_assessment(run: &Run, git: &dyn Git, purpose: AssessmentPurpose) -> CheckSelection {
    let mode = match purpose {
        AssessmentPurpose::Dashboard => match run.mode {
            None => {
                return CheckSelection::Unavailable(
                    "Record the workflow mode to select checks".into(),
                );
            }
            Some(Mode::Fast) => {
                return CheckSelection::Selected {
                    mode: Mode::Fast,
                    selection: checks::select(Mode::Fast, &[] as &[String]),
                };
            }
            Some(mode) => Some(mode),
        },
        AssessmentPurpose::Guidance
            if run.stage == Stage::Check && run.status == RunStatus::Done =>
        {
            return match run.mode.filter(|&mode| mode != Mode::Fast) {
                Some(mode) => CheckSelection::Selected {
                    mode,
                    selection: checks::select(mode, &[] as &[String]),
                },
                None => CheckSelection::NotApplicable,
            };
        }
        AssessmentPurpose::Advance if run.status != RunStatus::Running => None,
        AssessmentPurpose::Advance | AssessmentPurpose::Guidance
            if run.stage == Stage::Check && run.status != RunStatus::Done =>
        {
            run.mode.filter(|&mode| mode != Mode::Fast)
        }
        AssessmentPurpose::Advance | AssessmentPurpose::Guidance => None,
    };
    let Some(mode) = mode else {
        return CheckSelection::NotApplicable;
    };
    match select_checks(run, mode, git) {
        Ok(selection) => CheckSelection::Selected { mode, selection },
        Err(reason) => CheckSelection::Unavailable(reason),
    }
}

fn select_checks(run: &Run, mode: Mode, git: &dyn Git) -> Result<Selection, String> {
    changed_paths(run, git).map(|paths| checks::select(mode, &paths))
}

fn changed_paths(run: &Run, git: &dyn Git) -> Result<Vec<String>, String> {
    let worktree = run
        .worktree
        .as_ref()
        .ok_or("Record the worktree to select checks")?;
    let base = run
        .base_commit
        .as_ref()
        .ok_or("Record the base commit to select checks")?;
    let range = format!("{base}..HEAD");
    git.git(
        Path::new(worktree),
        &["diff", "--name-only", "--no-renames", "-z", &range],
    )
    .map(|out| out.split_terminator('\0').map(str::to_string).collect())
    .map_err(|error| format!("Could not inspect the changed paths in {worktree}: {error}"))
}

fn evaluate(
    run: &Run,
    gates: &[Gate],
    git: &dyn Git,
    candidate: &Option<Result<String, String>>,
    selection: &CheckSelection,
) -> Vec<Evaluation> {
    let candidate = candidate
        .as_ref()
        .ok_or("This stage has no local candidate")
        .and_then(|head| head.as_deref().map_err(String::as_str));
    gates
        .iter()
        .map(|&gate| evaluate_gate(run, gate, git, candidate, selection))
        .collect()
}

fn candidate(run: &Run, gates: &[Gate], git: &dyn Git) -> Option<Result<String, String>> {
    gates
        .iter()
        .any(|gate| {
            matches!(
                gate,
                Gate::Verified | Gate::TicketsVerified | Gate::ChecksRecorded
            )
        })
        .then(|| run.worktree_head(git))
}

/// Relevant recorded results for each subject in a failed or incomplete gate.
fn gate_records<'a>(run: &'a Run, gate: Gate, selection: &CheckSelection) -> Vec<&'a GateResult> {
    match gate {
        Gate::ChecksRecorded => match selection {
            CheckSelection::Selected { selection, .. } => selection
                .applicable
                .iter()
                .filter(|entry| entry.check.gate.gate() == gate)
                .filter_map(|entry| latest_check(run, gate, entry.check.name))
                .collect(),
            CheckSelection::NotApplicable | CheckSelection::Unavailable(_) => Vec::new(),
        },
        Gate::TicketsVerified => match run.unticketed_attempt {
            Some(attempt) => unticketed_verification(run, attempt).into_iter().collect(),
            None => run
                .tickets
                .iter()
                .filter_map(|ticket| latest_ticket_verification(run, ticket))
                .collect(),
        },
        Gate::Verified | Gate::NoActionableFindings => latest(run, gate).into_iter().collect(),
        Gate::TicketsDone | Gate::FinishSummary | Gate::CiPassed => Vec::new(),
    }
}

fn evaluate_gate(
    run: &Run,
    gate: Gate,
    git: &dyn Git,
    candidate: Result<&str, &str>,
    selection: &CheckSelection,
) -> Evaluation {
    let mut evaluation = Evaluation::with_status(gate, GateStatus::Passed);
    let result = match gate {
        Gate::TicketsDone => tickets_done(run).map(|()| {
            evaluation.detail = run
                .unticketed_attempt
                .map(|attempt| format!("Unticketed work (attempt {attempt})"))
                .or_else(|| {
                    Some(
                        run.tickets
                            .iter()
                            .map(|ticket| {
                                format!("{} (attempt {})", ticket.reference, ticket.work_attempt)
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                });
            Vec::new()
        }),
        Gate::Verified => passing_record(latest(run, gate)).and_then(|result| {
            require(
                result.commit.as_deref() == Some(candidate?),
                "The verification commit does not match HEAD",
            )?;
            Ok(vec![result])
        }),
        Gate::TicketsVerified => candidate
            .map_err(str::to_owned)
            .and_then(|head| tickets_verified(run, git, head)),
        Gate::ChecksRecorded => match selection {
            CheckSelection::Selected { selection, .. } => candidate
                .map_err(str::to_owned)
                .and_then(|head| check_results(run, selection, gate, head)),
            CheckSelection::Unavailable(reason) => Err(reason.clone()),
            CheckSelection::NotApplicable => Err("This stage has no check selection".into()),
        },
        Gate::NoActionableFindings => no_open_findings(run)
            .and_then(|()| passing_record(latest(run, gate)).map(|result| vec![result])),
        Gate::FinishSummary => run.check_finish_summary().map(|()| {
            evaluation.detail = run.summaries.get(&Stage::Finish).cloned();
            Vec::new()
        }),
        Gate::CiPassed => {
            return Evaluation::with_status(gate, GateStatus::AwaitingLiveCheck);
        }
    };
    match result {
        Ok(results) => evaluation.results = results.into_iter().cloned().collect(),
        Err(reason) => {
            evaluation.reason = Some(reason);
            let failed: Vec<_> = gate_records(run, gate, selection)
                .into_iter()
                .filter(|result| !result.passed)
                .cloned()
                .collect();
            evaluation.status = if failed.is_empty() {
                GateStatus::Open
            } else {
                GateStatus::Failed
            };
            evaluation.results = failed;
        }
    }
    evaluation
}

fn passing_record(recorded: Option<&GateResult>) -> Result<&GateResult, String> {
    recorded
        .filter(|record| record.passed)
        .ok_or_else(|| "No passing result was recorded".into())
}

impl Gate {
    pub(crate) fn remedy(self) -> &'static str {
        match self {
            Self::TicketsDone => {
                r#"list tickets with `jidoka run update <id> --ticket "<ref> <title>"` and mark them done with `--ticket-status <ref>=done`, or declare `--unticketed`"#
            }
            Self::Verified => {
                "run full verification and record `jidoka run gate <id> verified pass`"
            }
            Self::TicketsVerified => {
                "mark the listed ticket in progress or declare unticketed work, then verify and run `jidoka run gate <id> tickets-verified pass`"
            }
            Self::ChecksRecorded => {
                "record each result with `jidoka run gate <id> checks-recorded pass --check <name>`"
            }
            Self::NoActionableFindings => RECORD_OR_RESOLVE_FINDINGS,
            Self::FinishSummary => "record `jidoka run update <id> --summary finish=<text>`",
            Self::CiPassed => "open the pull request and pass CI; run `jidoka pr wait <id>`",
        }
    }
}

/// Checks the exit gates of the current Stage, and CI when leaving Finish, and returns the accepted exit.
///
/// # Errors
///
/// Returns a message naming each failing gate and its remedy, or why CI has not passed.
pub(crate) fn accepted_exit(run: &Run, git: &dyn Git, host: &dyn PrHost) -> Result<Exit, String> {
    let LocalAssessment {
        transition,
        evaluations: gates,
        checks,
        candidate,
    } = assess_local(run, git, AssessmentPurpose::Advance);
    let transition = transition?;
    let failures: Vec<_> = gates
        .iter()
        .filter_map(|gate| {
            gate.reason
                .as_ref()
                .map(|reason| gate_failure(run, gate.name, reason))
        })
        .collect();
    if !failures.is_empty() {
        return Err(format!("Cannot advance:\n{}", failures.join("\n")));
    }
    if let Some(Ok(before)) = candidate
        && run.worktree_head(git)? != before
    {
        return Err("The worktree HEAD changed while checking gates; try again".into());
    }

    if transition.to == Destination::Done {
        let ci = ci_passed(run, git, host).map_err(|reason| {
            format!(
                "Cannot advance:\n{}",
                gate_failure(run, Gate::CiPassed, &reason)
            )
        })?;
        return Ok(Exit::Finished { gates, ci });
    }
    Ok(Exit::Accepted {
        gates,
        skipped_checks: match checks {
            CheckSelection::Selected { selection, .. } => skipped_snapshot(&selection),
            CheckSelection::NotApplicable | CheckSelection::Unavailable(_) => Vec::new(),
        },
    })
}

fn gate_failure(run: &Run, gate: Gate, reason: &str) -> String {
    format!(
        "{}: {reason}; {}",
        gate.name(),
        gate.remedy().replace("<id>", &run.id)
    )
}

/// The requested stage transition and metadata to save with it.
#[derive(Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdvanceRequest {
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<String>,
    #[serde(flatten)]
    summaries: SummariesUpdate,
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
            None => accepted_exit(run, git, host),
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

fn require(condition: bool, reason: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(reason.to_string())
    }
}

fn ci_passed(run: &Run, git: &dyn Git, host: &dyn PrHost) -> Result<CiEvidence, String> {
    let url = run.pr.as_deref().ok_or("No pull request is recorded")?;
    let checked_at = updated_now().map_err(|error| format!("Could not read the time: {error}"))?;
    let commit = run.worktree_head(git)?;
    let options = pr_wait::Options {
        timeout: CI_TIMEOUT,
        pushed_head: None,
        ready_only: true,
        ..Default::default()
    };
    let clock = pr_wait::RealClock(Instant::now());
    let result = host
        .watch(url, &options, &clock, &mut |_| {}, &mut |_| {})
        .map_err(|error| format!("Could not check CI: {}", error.0))?;
    require(
        result.state == pr_state::PrState::Open,
        &format!(
            "Pull request is {}; if it merged, run `jidoka run sync {}` to reconcile the run",
            result.state.label(),
            run.id
        ),
    )?;
    require(
        result.head == commit,
        "The pull request head does not match the worktree HEAD",
    )?;
    let outcome = match result.outcome {
        pr_wait::Outcome::Passed => CiOutcome::Passed,
        pr_wait::Outcome::NoCi => CiOutcome::NoCi,
        other => return Err(format!("CI outcome is {other:?}")),
    };
    require(
        run.worktree_head(git)? == commit,
        "The worktree HEAD changed while checking CI; try again",
    )?;
    Ok(CiEvidence {
        head: commit,
        outcome,
        pr_status: pr_state::PrStatus {
            url: url.into(),
            state: result.state,
            reason: None,
            checked_at,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pr_host::{FakeHost, Head},
        pr_wait::Outcome,
        run::{Completion, ExitOutcome, Mode, Ticket},
    };
    use std::cell::RefCell;

    /// Records a gate on a run value, as the store does with the result.
    fn record_on(run: &mut Run, gate: Record, git: &dyn Git) -> Result<(), String> {
        let result = record(run, gate, git)?;
        run.gates.push(result);
        Ok(())
    }

    /// Leaves the Stage of a run value once its exit gates pass, as the store does with the exit.
    fn advance_on(run: &mut Run, git: &dyn Git, tool: &dyn PrHost) -> Result<(), String> {
        let exit = accepted_exit(run, git, tool)?;
        run.leave(exit, 1)
    }

    /// Advances run 1 through the store once its exit gates pass.
    fn advance_stored(
        store: &crate::run::RunStore,
        git: &dyn Git,
        tool: &dyn PrHost,
    ) -> Result<(), String> {
        store.advance("1", AdvanceMetadata::default(), |run| {
            accepted_exit(run, git, tool)
        })
    }

    fn force(run: &mut Run, reason: &str) -> Result<(), String> {
        run.leave(
            Exit::Override {
                reason: reason.into(),
            },
            1,
        )
    }

    struct FakeGit(RefCell<String>);
    impl Git for FakeGit {
        fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
            Ok(self.0.borrow().clone())
        }

        fn status(&self, _: &Path, args: &[&str]) -> Result<i32, String> {
            Ok(match args {
                ["merge-base", "--is-ancestor", "abc1234", "def5678"] => 0,
                ["merge-base", "--is-ancestor", _, _] => 1,
                _ => panic!("Unexpected git status command {args:?}"),
            })
        }
    }
    fn run(stage: Stage) -> Run {
        let mut run: Run = serde_json::from_value(serde_json::json!({"gates":[],"id":"1","title":"t","project":"/p","stage":stage,"status":"running"})).unwrap();
        run.mode = Some(Mode::Fast);
        run.worktree = Some("/work".into());
        run.base_commit = Some("abc1234".into());
        run.pr = Some("https://github.com/o/r/pull/7".into());
        run.summaries
            .insert(Stage::Finish, "Implemented the task".into());
        run
    }
    fn result(gate: Gate, passed: bool) -> Record {
        Record {
            gate,
            passed,
            evidence: None,
            check: (gate == Gate::ChecksRecorded).then(|| "code-review".to_owned()),
        }
    }
    /// An open pull request at commit `abc1234` whose CI passes.
    fn host() -> FakeHost {
        FakeHost::states(&[pr_state::PrState::Open])
    }

    /// An open pull request at `commit` whose CI passes.
    fn host_at(state: pr_state::PrState, commit: &str) -> FakeHost {
        FakeHost::new(vec![Ok(Head {
            state,
            commit: commit.into(),
            branch: "feature".into(),
        })])
    }

    #[test]
    fn advance_keeps_usage_collected_meanwhile_but_refuses_other_changes() {
        use crate::run::RunStore;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runs.json");
        std::fs::write(&path, serde_json::to_vec(&vec![run(Stage::Build)]).unwrap()).unwrap();
        let store = RunStore::open(&path).unwrap();
        let usage: crate::usage::Usage =
            serde_json::from_value(serde_json::json!({"status":"tracked","collected_at":7}))
                .unwrap();
        let forced = || Exit::Override {
            reason: "Tickets done by hand".into(),
        };
        store
            .advance("1", AdvanceMetadata::default(), |_| {
                store.set_usage("1", usage.clone()).unwrap();
                Ok(forced())
            })
            .unwrap();
        let saved = store.get("1").unwrap().unwrap();
        assert_eq!(saved.stage, Stage::Finish);
        assert_eq!(saved.usage, Some(usage));
        assert!(!saved.run_reports.is_empty());

        let refused = store.advance("1", AdvanceMetadata::default(), |_| {
            let update = crate::run::parse_update(&["--activity", "Fixing CI"]).unwrap();
            store.update("1", update).unwrap();
            Ok(forced())
        });
        assert_eq!(
            refused.unwrap_err(),
            "The run changed while checking gates; try again"
        );
        let kept = store.get("1").unwrap().unwrap();
        assert_eq!(
            (kept.stage, kept.status),
            (Stage::Finish, RunStatus::Running)
        );
        assert_eq!(kept.activity.as_deref(), Some("Fixing CI"));
    }

    #[test]
    fn forced_advance_skips_gates_records_the_reason_and_needs_one() {
        let mut run = run(Stage::Build);
        force(&mut run, "Tickets done by hand").unwrap();
        assert_eq!(run.stage, Stage::Finish);
        assert_eq!(
            run.run_reports.last().unwrap().text,
            "Forced advance from build: Tickets done by hand"
        );
        let request = |force, reason: Option<&str>| AdvanceRequest {
            force,
            reason: reason.map(String::from),
            ..AdvanceRequest::default()
        };
        assert!(request(true, None).forced_reason().is_err());
        assert!(request(true, Some(" ")).forced_reason().is_err());
        assert!(request(false, Some("why")).forced_reason().is_err());
        assert_eq!(
            request(true, Some("why")).forced_reason().unwrap(),
            Some("why")
        );
        assert_eq!(request(false, None).forced_reason().unwrap(), None);
    }

    #[test]
    fn advance_options_are_read_in_any_order_and_refused_when_wrong() {
        let parsed = parse_advance_options(&[
            "--json",
            "--activity",
            "Checking",
            "--summary",
            "build=Done",
        ])
        .unwrap();
        assert!(parsed.json && !parsed.request.force);
        let body = serde_json::to_value(parsed.request).unwrap();
        assert_eq!(body["activity"], "Checking");
        assert_eq!(body["summaries"]["build"], "Done");
        assert_eq!(
            parse_advance_options(&["--force", "--reason", "why"])
                .unwrap()
                .request,
            AdvanceRequest {
                force: true,
                reason: Some("why".into()),
                ..AdvanceRequest::default()
            }
        );
        assert_eq!(
            parse_advance_options(&[]).unwrap(),
            AdvanceOptions::default()
        );
        for bad in [
            &["--bogus"][..],
            &["--activity"],
            &["--json", "--json"],
            &["--force"],
            &["--reason", "why"],
            &["--summary", "a=b", "--summary", "c=d"],
        ] {
            assert!(parse_advance_options(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn build_requires_tickets_and_verification_of_current_head() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Build);
        run.tickets.push(Ticket {
            reference: "1".into(),
            title: "one".into(),
            status: TicketStatus::Pending,
            work_attempt: 0,
        });
        let error = advance_on(&mut run, &git, &host()).unwrap_err();
        assert!(error.contains("tickets-done") && error.contains("verified"));
        record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
        run.tickets[0].status = TicketStatus::Done;
        *git.0.borrow_mut() = "def5678".into();
        assert!(
            advance_on(&mut run, &git, &host())
                .unwrap_err()
                .contains("verified")
        );
        record_on(&mut run, result(Gate::Verified, false), &git).unwrap();
        assert!(advance_on(&mut run, &git, &host()).is_err());
        record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
        advance_on(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Finish);
    }
    #[test]
    fn full_build_requires_a_passing_result_for_each_ticket() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Build);
        run.mode = Some(Mode::Full);
        for reference in ["one", "two"] {
            run.tickets.push(Ticket {
                reference: reference.into(),
                title: reference.into(),
                status: TicketStatus::Pending,
                work_attempt: 0,
            });
        }
        assert!(
            advance_on(&mut run, &git, &host())
                .unwrap_err()
                .contains("tickets-verified")
        );
        assert!(record_on(&mut run, result(Gate::TicketsVerified, true), &git).is_err());
        run.tickets[0].status = TicketStatus::InProgress;
        record_on(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
        run.tickets[0].status = TicketStatus::Done;
        assert!(advance_on(&mut run, &git, &host()).is_err());
        run.tickets[1].status = TicketStatus::InProgress;
        record_on(&mut run, result(Gate::TicketsVerified, false), &git).unwrap();
        assert!(advance_on(&mut run, &git, &host()).is_err());
        record_on(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
        run.tickets[1].status = TicketStatus::Done;
        advance_on(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Check);
    }

    #[test]
    fn full_run_walks_stages_and_rework_with_ticket_verification() {
        use crate::run::{RunStore, parse_update};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runs.json");
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut initial = run(Stage::Define);
        initial.mode = Some(Mode::Full);
        std::fs::write(&path, serde_json::to_vec(&vec![initial]).unwrap()).unwrap();
        let store = RunStore::open(&path).unwrap();
        let update = |options: &[&str]| {
            store
                .update_with("1", parse_update(options).unwrap(), &git)
                .unwrap()
                .unwrap();
        };
        let gate = |result| store.record_gate("1", |run| record(run, result, &git));
        let gh = host_at(pr_state::PrState::Open, "def5678");
        let advance = || advance_stored(&store, &git, &gh);
        update(&[
            "--ticket",
            "https://example.com/1 One",
            "--ticket",
            "https://example.com/2 Two",
        ]);
        advance().unwrap();
        let current = store.get("1").unwrap().unwrap();
        let next = serde_json::to_value(crate::run_next::build(&current, &git).unwrap()).unwrap();
        assert!(
            next["instructions"]
                .as_str()
                .unwrap()
                .contains("tickets-verified pass")
        );
        for (reference, commit) in [
            ("https://example.com/1", "abc1234"),
            ("https://example.com/2", "def5678"),
        ] {
            *git.0.borrow_mut() = commit.into();
            update(&["--ticket-status", &format!("{reference}=in-progress")]);
            gate(result(Gate::TicketsVerified, true)).unwrap();
            let current = store.get("1").unwrap().unwrap();
            let active = current.active_ticket().unwrap();
            assert_eq!(
                current.timeline.last().unwrap().ticket.as_deref(),
                Some(reference)
            );
            let verified = latest_ticket_verification(&current, active).unwrap();
            assert_eq!(verified.commit.as_deref(), Some(commit));
            update(&["--ticket-status", &format!("{reference}=done")]);
        }
        *git.0.borrow_mut() = "other123".into();
        let error = advance().unwrap_err().to_string();
        assert!(error.contains("outside the current Git history"), "{error}");
        *git.0.borrow_mut() = "def5678".into();
        advance().unwrap();
        let first_build = store.get("1").unwrap().unwrap().stage_exits[1].clone();
        update(&["--finding", "code-review", "-", "Bug"]);
        store
            .transition(
                "1",
                crate::run::parse_lifecycle("rework", &["--finding", "F1=behavior"]).unwrap(),
            )
            .unwrap();
        let reopened = "https://example.com/2";
        update(&["--ticket-status", &format!("{reopened}=in-progress")]);
        update(&["--ticket-status", &format!("{reopened}=done")]);
        let error = advance().unwrap_err().to_string();
        assert!(
            error.contains(reopened) && error.contains("current attempt"),
            "{error}"
        );
        update(&["--ticket-status", &format!("{reopened}=in-progress")]);
        gate(result(Gate::TicketsVerified, true)).unwrap();
        update(&["--ticket-status", &format!("{reopened}=done")]);
        advance().unwrap();
        gate(result(Gate::ChecksRecorded, true)).unwrap();
        let error = advance().unwrap_err();
        assert!(error.contains("F1 are open"), "{error}");
        assert!(gate(result(Gate::NoActionableFindings, true)).is_err());
        update(&["--finding-status", "F1=fixed"]);
        gate(result(Gate::NoActionableFindings, true)).unwrap();
        gate(result(Gate::Verified, true)).unwrap();
        advance().unwrap();
        advance().unwrap();
        let finished = store.get("1").unwrap().unwrap();
        assert_eq!(finished.status, RunStatus::Done);
        assert_eq!(finished.stage_exits[1], first_build);
        assert!(matches!(
            finished.stage_exits[2].outcome,
            ExitOutcome::Rework { number: 1, .. }
        ));
        assert_ne!(finished.stage_exits[1].gates, finished.stage_exits[3].gates);
    }

    #[test]
    fn a_finding_opened_after_review_blocks_next_and_advance() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        for gate in [
            Gate::ChecksRecorded,
            Gate::NoActionableFindings,
            Gate::Verified,
        ] {
            record_on(&mut run, result(gate, true), &git).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        std::fs::write(&path, serde_json::to_vec(&[&run]).unwrap()).unwrap();
        let store = crate::run::RunStore::open(&path).unwrap();
        let update = crate::run::parse_update(&["--finding", "code-review", "-", "Bug"]).unwrap();
        let run = store.update_with("1", update, &git).unwrap().unwrap();
        let next = crate::run_next::build(&run, &git).unwrap();
        let text = crate::run_next::render(&next);
        assert!(
            text.contains("Findings F1 are open"),
            "Missing open Finding in guidance: {text}"
        );
        assert!(
            text.contains("Move to build (rework)"),
            "Missing Rework move in guidance: {text}"
        );
        let dashboard = serde_json::to_value(crate::run_next::stage_gates(
            &run,
            assess_local(&run, &git, AssessmentPurpose::Dashboard).evaluations,
        ))
        .unwrap();
        assert!(dashboard.to_string().contains("Findings F1 are open"));
        let error = advance_stored(&store, &git, &host()).unwrap_err();
        assert!(error.contains("Findings F1 are open"));
        assert!(store.get("1").unwrap().unwrap().stage_exits.is_empty());
    }

    #[test]
    fn named_gate_failures_show_their_evidence_for_the_current_pass() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        for (stage, gate, subject) in [
            (Stage::Check, Gate::ChecksRecorded, "code-review"),
            (Stage::Build, Gate::TicketsVerified, "local-1"),
        ] {
            let mut run = run(stage);
            run.mode = Some(Mode::Full);
            if stage == Stage::Build {
                run.tickets.push(Ticket {
                    reference: subject.into(),
                    title: "Work".into(),
                    status: TicketStatus::InProgress,
                    work_attempt: 1,
                });
            }
            let mut failed = result(gate, false);
            failed.evidence = Some("Verification timed out".into());
            record_on(&mut run, failed, &git).unwrap();
            let views = |run: &Run| {
                serde_json::to_value(crate::run_next::stage_gates(
                    run,
                    assess_local(run, &git, AssessmentPurpose::Dashboard).evaluations,
                ))
                .unwrap()
            };
            let evaluated = |views: serde_json::Value| {
                views
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["stage"] == stage.name())
                    .unwrap()["gates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|result| result["name"] == gate.name())
                    .unwrap()
                    .clone()
            };
            let result = evaluated(views(&run));
            assert_eq!(result["status"], "failed");
            let field = if stage == Stage::Build {
                "ticket"
            } else {
                "check"
            };
            assert_eq!(result["results"][0][field], subject);
            assert_eq!(result["results"][0]["evidence"], "Verification timed out");
            if stage == Stage::Build {
                run.tickets[0].work_attempt += 1;
            } else {
                run.rework = Some(1);
            }
            assert_eq!(evaluated(views(&run))["status"], "open");
        }
    }

    #[test]
    fn full_unticketed_build_requires_a_declaration_and_a_pass_on_the_current_head() {
        use crate::run::{RunStore, parse_update};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runs.json");
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut initial = run(Stage::Build);
        initial.mode = Some(Mode::Full);
        std::fs::write(&path, serde_json::to_vec(&vec![initial]).unwrap()).unwrap();
        let store = RunStore::open(&path).unwrap();
        let update = |options: &[&str]| {
            store
                .update_with("1", parse_update(options).unwrap(), &git)
                .unwrap();
        };
        let advance = || advance_stored(&store, &git, &host());
        let pass = || {
            store.record_gate("1", |run| {
                record(run, result(Gate::TicketsVerified, true), &git)
            })
        };

        let error = advance().unwrap_err().to_string();
        assert!(
            error.contains("tickets-done") && error.contains("no ticket plan"),
            "{error}"
        );
        let error = pass().unwrap_err();
        assert!(
            error.contains("Mark a listed ticket in progress")
                && error.contains("declare unticketed work"),
            "{error}"
        );
        update(&["--unticketed"]);
        let error = advance().unwrap_err().to_string();
        assert!(
            error.contains("tickets-verified") && !error.contains("tickets-done"),
            "{error}"
        );
        pass().unwrap();
        *git.0.borrow_mut() = "def5678".into();
        assert!(advance().is_err(), "a moved HEAD needs fresh evidence");
        pass().unwrap();
        update(&["--clear-tickets"]);
        update(&["--unticketed"]);
        assert!(advance().is_err(), "a replaced plan needs fresh evidence");
        pass().unwrap();
        advance().unwrap();
    }

    #[test]
    fn merged_pull_request_advise_sync_instead_of_accepting_unchecked_ci() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Finish);
        let error = advance_on(
            &mut run,
            &git,
            &FakeHost::states(&[pr_state::PrState::Merged]),
        )
        .unwrap_err();
        assert!(error.contains("jidoka run sync 1"));
        assert_eq!(run.status, RunStatus::Running);
    }

    #[test]
    fn merge_during_ci_check_reports_sync_even_if_the_head_changed() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Finish);
        let merged = host_at(pr_state::PrState::Merged, "new-head");
        let error = advance_on(&mut run, &git, &merged).unwrap_err();
        assert!(error.contains("jidoka run sync 1"), "{error}");
        assert_eq!(run.status, RunStatus::Running);
    }

    #[test]
    fn finish_checks_pr_state_ci_and_head() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        for host in [
            FakeHost::states(&[pr_state::PrState::Closed]),
            host().with_ci(Outcome::Failed),
            host_at(pr_state::PrState::Open, "def5678"),
        ] {
            assert!(advance_on(&mut run(Stage::Finish), &git, &host).is_err());
        }
        let mut run = run(Stage::Finish);
        advance_on(&mut run, &git, &host()).unwrap();
        assert_eq!(run.status, RunStatus::Done);
    }
    #[test]
    fn review_modes_verify_the_final_commit_after_review_and_rework() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        for mode in [Mode::Balanced, Mode::Full] {
            *git.0.borrow_mut() = "abc1234".into();
            let mut run = run(Stage::Build);
            run.mode = Some(mode);
            run.rework = Some(1);
            run.tickets.push(Ticket {
                reference: "ticket".into(),
                title: "ticket".into(),
                status: TicketStatus::Done,
                work_attempt: 0,
            });
            if mode == Mode::Full {
                run.tickets[0].status = TicketStatus::InProgress;
                record_on(&mut run, result(Gate::TicketsVerified, true), &git).unwrap();
                run.tickets[0].status = TicketStatus::Done;
            }
            advance_on(&mut run, &git, &host()).unwrap();
            assert_eq!(run.stage, Stage::Check);
            record_on(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
            record_on(&mut run, result(Gate::NoActionableFindings, true), &git).unwrap();
            assert!(
                advance_on(&mut run, &git, &host())
                    .unwrap_err()
                    .contains("verified")
            );
            record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
            *git.0.borrow_mut() = "def5678".into();
            assert!(
                advance_on(&mut run, &git, &host())
                    .unwrap_err()
                    .contains("verified")
            );
            record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
            assert!(
                advance_on(&mut run, &git, &host())
                    .unwrap_err()
                    .contains("checks-recorded")
            );
            record_on(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
            advance_on(&mut run, &git, &host()).unwrap();
            assert_eq!(run.stage, Stage::Finish);
        }
    }
    #[test]
    fn finish_requires_a_summary_even_when_forced() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Finish);
        run.summaries.insert(Stage::Finish, "  ".into());
        assert!(
            advance_on(&mut run, &git, &host())
                .unwrap_err()
                .contains("--summary finish=")
        );
        assert!(force(&mut run, "CI service unavailable").is_err());
        assert_eq!(run.status, RunStatus::Running);
        assert!(run.run_reports.is_empty());
        assert!(run.gates.is_empty());

        run.summaries
            .insert(Stage::Finish, "Implemented the task".into());
        force(&mut run, "CI service unavailable").unwrap();
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(
            run.completion,
            Some(Completion::Override {
                reason: "CI service unavailable".into()
            })
        );
        assert!(run.gates.is_empty());
    }

    #[test]
    fn advancing_out_of_finish_stores_the_ci_result() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runs.json");
        let initial = run(Stage::Finish);
        std::fs::write(&path, serde_json::to_vec(&vec![initial]).unwrap()).unwrap();
        let store = crate::run::RunStore::open(&path).unwrap();
        advance_stored(&store, &git, &host()).unwrap();
        let run = store.get("1").unwrap().unwrap();
        let stored = run.gates.iter().find(|r| r.gate == Gate::CiPassed).unwrap();
        assert!(stored.passed);
        assert_eq!(stored.stage, Stage::Finish);
        assert_eq!(stored.evidence.as_deref(), Some("CI passed on abc1234"));
        assert_eq!(
            run.completion,
            Some(Completion::Ci {
                head: "abc1234".into(),
                outcome: CiOutcome::Passed
            })
        );
        assert_eq!(
            run.pr_status.as_ref().map(|status| status.state),
            Some(pr_state::PrState::Open)
        );
        assert!(crate::inspect::render(&run).contains("CI passed on abc1234"));
        assert!(crate::inspect::render(&run).contains("PR state      open"));
        let mut merged = run.pr_status.clone().unwrap();
        merged.state = pr_state::PrState::Merged;
        store.record_pr_result("1", merged).unwrap();
        let reopened = crate::run::RunStore::open(&path).unwrap();
        let after_merge = reopened.get("1").unwrap().unwrap();
        assert_eq!(after_merge.completion, run.completion);
        assert_eq!(after_merge.stage_exits, run.stage_exits);
        assert_eq!(after_merge.finished_at, run.finished_at);
        assert_eq!(
            after_merge.pr_status.unwrap().state,
            pr_state::PrState::Merged
        );
    }

    #[test]
    fn finishing_without_ci_records_no_ci() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Finish);
        advance_on(&mut run, &git, &host().with_ci(Outcome::NoCi)).unwrap();
        assert_eq!(run.status, RunStatus::Done);
        assert_eq!(
            run.completion,
            Some(Completion::Ci {
                head: "abc1234".into(),
                outcome: CiOutcome::NoCi
            })
        );
    }

    struct BrokenGit;
    impl Git for BrokenGit {
        fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
            Err("repository unavailable".into())
        }
    }
    #[test]
    fn failed_external_checks_report_the_cause_and_remedy() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut build = run(Stage::Build);
        record_on(&mut build, result(Gate::Verified, true), &git).unwrap();
        let error = advance_on(&mut build, &BrokenGit, &host()).unwrap_err();
        assert!(
            error.contains("repository unavailable") && error.contains("jidoka run gate"),
            "{error}"
        );
        let broken = FakeHost::new(vec![Err("host unavailable".into())]);
        let error = advance_on(&mut run(Stage::Finish), &git, &broken).unwrap_err();
        assert!(
            error.contains("host unavailable") && error.contains("jidoka pr wait"),
            "{error}"
        );
    }

    #[test]
    fn check_requires_recorded_passing_results() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        record_on(&mut run, result(Gate::ChecksRecorded, true), &git).unwrap();
        record_on(&mut run, result(Gate::NoActionableFindings, false), &git).unwrap();
        assert!(advance_on(&mut run, &git, &host()).is_err());
        record_on(&mut run, result(Gate::NoActionableFindings, true), &git).unwrap();
        record_on(&mut run, result(Gate::Verified, true), &git).unwrap();
        advance_on(&mut run, &git, &host()).unwrap();
        assert_eq!(run.stage, Stage::Finish);
    }
    #[test]
    fn check_results_are_independent_and_bound_to_the_reviewed_head() {
        static A: crate::checks::Check = crate::checks::Check {
            name: "a",
            triggers: &[],
            instructions: "a",
            gate: crate::checks::CheckGate::Recorded,
            model_route: None,
            floor: &[],
        };
        static B: crate::checks::Check = crate::checks::Check {
            name: "b",
            triggers: &[],
            instructions: "b",
            gate: crate::checks::CheckGate::Recorded,
            model_route: None,
            floor: &[],
        };
        let selection = crate::checks::Selection {
            applicable: vec![
                crate::checks::Applicable {
                    check: &A,
                    reason: crate::checks::Reason::Floor,
                },
                crate::checks::Applicable {
                    check: &B,
                    reason: crate::checks::Reason::Floor,
                },
            ],
            skipped: vec![],
        };
        let mut run = run(Stage::Check);
        let result = |name: &str, passed| GateResult {
            gate: Gate::ChecksRecorded,
            stage: Stage::Check,
            rework: None,
            passed,
            evidence: None,
            commit: Some("head-1".into()),
            ticket: None,
            work_attempt: None,
            check: Some(name.into()),
            skipped_checks: vec![],
        };
        let check = |run: &Run, head: &str| {
            check_results(run, &selection, Gate::ChecksRecorded, head).map(|results| results.len())
        };
        run.gates.push(result("a", true));
        assert!(check(&run, "head-1").is_err());
        run.gates.push(result("b", true));
        assert_eq!(check(&run, "head-1").unwrap(), 2);
        assert!(check(&run, "head-2").is_err());
        run.gates.push(result("b", false));
        assert!(check(&run, "head-1").is_err());
        run.rework = Some(1);
        assert!(check(&run, "head-1").is_err());
    }

    #[test]
    fn skipped_snapshot_survives_serialization_and_rework() {
        static EXTRA: crate::checks::Check = crate::checks::Check {
            name: "extra",
            triggers: &["*.tf"],
            instructions: "extra",
            gate: crate::checks::CheckGate::Recorded,
            model_route: None,
            floor: &[],
        };
        let selection = crate::checks::Selection {
            applicable: vec![],
            skipped: vec![crate::checks::Skipped {
                check: &EXTRA,
                reason: "no matching path".into(),
            }],
        };
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        run.leave(
            Exit::Accepted {
                gates: Vec::new(),
                skipped_checks: skipped_snapshot(&selection),
            },
            0,
        )
        .unwrap();
        let mut restored: Run =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        restored.rework = Some(1);
        assert_eq!(restored.stage_exits[0].skipped_checks[0].name, "extra");
        assert_eq!(
            restored.stage_exits[0].skipped_checks[0].reason,
            "no matching path"
        );
    }

    #[test]
    fn gate_options_accept_either_order_and_reject_duplicates() {
        let expected = (Some("code-review".into()), Some("reviewed".into()));
        assert_eq!(
            parse_options(&["--check", "code-review", "--evidence", "reviewed"]).unwrap(),
            expected
        );
        assert_eq!(
            parse_options(&["--evidence", "reviewed", "--check", "code-review"]).unwrap(),
            expected
        );
        assert!(parse_options(&["--check", "a", "--check", "b"]).is_err());
    }

    #[test]
    fn recording_requires_an_applicable_named_check() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        let mut gate = result(Gate::ChecksRecorded, true);
        gate.check = None;
        assert!(
            record_on(&mut run, gate, &git)
                .unwrap_err()
                .contains("--check")
        );
        let mut gate = result(Gate::ChecksRecorded, true);
        gate.check = Some("nonexistent".into());
        assert!(
            record_on(&mut run, gate, &git)
                .unwrap_err()
                .contains("not applicable")
        );
    }
    struct NullDiff;

    impl Git for NullDiff {
        fn git(&self, _dir: &Path, args: &[&str]) -> Result<String, String> {
            assert_eq!(
                args,
                ["diff", "--name-only", "--no-renames", "-z", "abc..HEAD"]
            );
            Ok(" leading.txt\0line\nbreak.tf\0ümlaut.tf\0".into())
        }
    }

    #[test]
    fn changed_paths_preserves_unusual_names() {
        let mut run = run(Stage::Check);
        run.worktree = Some("/work/repo".into());
        run.base_commit = Some("abc".into());
        assert_eq!(
            changed_paths(&run, &NullDiff).unwrap(),
            [" leading.txt", "line\nbreak.tf", "ümlaut.tf"]
        );
    }

    #[test]
    fn git_diff_preserves_unusual_paths_and_both_sides_of_renames() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(dir.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", dir.path().join("no-config"))
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        git(&["init", "-q"]);
        let commit = |message| {
            git(&[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                message,
            ])
        };
        std::fs::write(dir.path().join("security.tf"), "resource").unwrap();
        git(&["add", "--all"]);
        commit("base");
        let base = git(&["rev-parse", "HEAD"]);
        git(&["mv", "security.tf", "renamed.txt"]);
        std::fs::write(dir.path().join(" leading.txt"), "one").unwrap();
        std::fs::write(dir.path().join("ümlaut.tf"), "two").unwrap();
        git(&["add", "--all"]);
        commit("change");
        let mut run = run(Stage::Check);
        run.worktree = Some(dir.path().to_string_lossy().into_owned());
        run.base_commit = Some(base);
        let paths = changed_paths(&run, &crate::git::ReadOnlyGit).unwrap();
        assert!(paths.contains(&" leading.txt".to_string()));
        assert!(paths.contains(&"ümlaut.tf".to_string()));
        assert!(paths.contains(&"security.tf".to_string()));
        assert!(paths.contains(&"renamed.txt".to_string()));
    }

    #[test]
    fn stale_check_evidence_blocks_guidance_and_advance() {
        let git = FakeGit(RefCell::new("abc1234".into()));
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        for gate in [
            Gate::ChecksRecorded,
            Gate::NoActionableFindings,
            Gate::Verified,
        ] {
            record_on(&mut run, result(gate, true), &git).unwrap();
        }
        let next = crate::run_next::build(&run, &git).unwrap();
        let next_json = serde_json::to_value(next).unwrap();
        let gates = &next_json["gates"];
        assert!(
            gates.as_array().unwrap().is_empty(),
            "Expected fresh evidence to satisfy every Gate, found {gates}"
        );

        *git.0.borrow_mut() = "def5678".into();
        let next = serde_json::to_value(crate::run_next::build(&run, &git).unwrap()).unwrap();
        let dashboard = assess_local(&run, &git, AssessmentPurpose::Dashboard);
        let error = advance_on(&mut run, &git, &host()).unwrap_err();
        for (gate, reason) in [
            (
                Gate::ChecksRecorded,
                "Check code-review has no passing result on HEAD",
            ),
            (
                Gate::Verified,
                "The verification commit does not match HEAD",
            ),
        ] {
            assert!(
                next["gates"].as_array().unwrap().iter().any(|entry| {
                    entry["name"] == gate.name()
                        && entry["status"] == "open"
                        && entry["reason"] == reason
                }),
                "Missing {gate:?} in guidance: {next}"
            );
            assert!(
                dashboard.evaluations.iter().any(|entry| {
                    entry.name == gate
                        && entry.status == GateStatus::Open
                        && entry.reason.as_deref() == Some(reason)
                }),
                "Missing {gate:?} in dashboard assessment"
            );
            assert!(
                error.contains(&format!("{}: {reason}", gate.name())),
                "{error}"
            );
        }
        assert_eq!(run.stage, Stage::Check);
        assert!(
            run.stage_exits.is_empty(),
            "Refused Advance saved Stage exits: {:?}",
            run.stage_exits
        );
    }

    #[test]
    fn advance_rejects_head_change_during_check_selection() {
        struct MovingHead(std::cell::Cell<u32>);
        impl Git for MovingHead {
            fn git(&self, _: &Path, args: &[&str]) -> Result<String, String> {
                if args.starts_with(&["rev-parse", "HEAD"]) {
                    let count = self.0.get();
                    self.0.set(count + 1);
                    Ok(if count == 0 { "old" } else { "new" }.into())
                } else {
                    Ok(String::new())
                }
            }
        }
        let steady = FakeGit(RefCell::new("old".into()));
        let mut run = run(Stage::Check);
        run.mode = Some(Mode::Balanced);
        record_on(&mut run, result(Gate::ChecksRecorded, true), &steady).unwrap();
        record_on(&mut run, result(Gate::NoActionableFindings, true), &steady).unwrap();
        record_on(&mut run, result(Gate::Verified, true), &steady).unwrap();
        let error =
            advance_on(&mut run, &MovingHead(std::cell::Cell::new(0)), &host()).unwrap_err();
        assert!(error.contains("HEAD changed"), "{error}");
        assert_eq!(run.stage, Stage::Check);
        assert!(run.stage_exits.is_empty());
    }

    #[test]
    fn independent_gate_records_merge_but_run_changes_reject_stale_validation() {
        use crate::run::{LifecycleRequest, RunStore, parse_update};
        struct Interleaved<'a> {
            store: &'a RunStore,
            id: &'a str,
            change_run: bool,
        }
        impl crate::git::Git for Interleaved<'_> {
            fn git(&self, _: &std::path::Path, _: &[&str]) -> Result<String, String> {
                if self.change_run {
                    self.store
                        .transition(
                            self.id,
                            LifecycleRequest::Block {
                                reason: "waiting".into(),
                            },
                        )
                        .unwrap();
                } else {
                    self.store
                        .record_gate(self.id, |run| {
                            record(
                                run,
                                Record {
                                    gate: Gate::NoActionableFindings,
                                    passed: true,
                                    evidence: None,
                                    check: None,
                                },
                                &crate::git::RealGit,
                            )
                        })
                        .unwrap();
                }
                Ok("abc123".into())
            }
        }
        let dir = crate::test_support::TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = crate::test_support::new_run(&store, "task");
        store
            .update(
                &run.id,
                parse_update(&["--worktree", "/work/repo"]).unwrap(),
            )
            .unwrap();
        let verified = || Record {
            gate: Gate::Verified,
            passed: true,
            evidence: None,
            check: None,
        };
        store
            .record_gate(&run.id, |run| {
                record(
                    run,
                    verified(),
                    &Interleaved {
                        store: &store,
                        id: &run.id,
                        change_run: false,
                    },
                )
            })
            .unwrap();
        let current = store.get(&run.id).unwrap().unwrap();
        assert_eq!(current.gates.len(), 2);
        assert!(
            current
                .gates
                .iter()
                .any(|gate| gate.gate == Gate::Verified && gate.commit.as_deref() == Some("abc123")),
            "verified gate should retain its checked commit"
        );
        assert!(
            current
                .gates
                .iter()
                .any(|gate| gate.gate == Gate::NoActionableFindings),
            "independent gate should be preserved"
        );
        let error = store
            .record_gate(&run.id, |run| {
                record(
                    run,
                    verified(),
                    &Interleaved {
                        store: &store,
                        id: &run.id,
                        change_run: true,
                    },
                )
            })
            .unwrap_err();
        assert!(
            error.contains("changed while checking"),
            "unexpected error: {error}"
        );
        assert_eq!(store.get(&run.id).unwrap().unwrap().gates.len(), 2);
    }

    #[test]
    fn gate_record_rejects_run_change_reverted_during_validation() {
        use crate::run::{RunStore, parse_update};

        struct Revert<'a> {
            store: &'a RunStore,
            id: &'a str,
        }
        impl crate::git::Git for Revert<'_> {
            fn git(&self, _: &std::path::Path, _: &[&str]) -> Result<String, String> {
                for activity in ["B", "A"] {
                    self.store
                        .update(self.id, parse_update(&["--activity", activity]).unwrap())
                        .unwrap();
                }
                Ok("abc123".into())
            }
        }
        let dir = crate::test_support::TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = crate::test_support::new_run(&store, "task");
        store
            .update(
                &run.id,
                parse_update(&["--worktree", "/work/repo", "--activity", "A"]).unwrap(),
            )
            .unwrap();
        let error = store
            .record_gate(&run.id, |run| {
                record(
                    run,
                    Record {
                        gate: Gate::Verified,
                        passed: true,
                        evidence: None,
                        check: None,
                    },
                    &Revert {
                        store: &store,
                        id: &run.id,
                    },
                )
            })
            .unwrap_err();
        assert!(
            error.contains("changed while checking"),
            "unexpected error: {error}"
        );
        assert!(
            store.get(&run.id).unwrap().unwrap().gates.is_empty(),
            "stale gate must not be recorded"
        );
    }
}
