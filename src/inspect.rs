//! The readable form of a run, for `jidoka run inspect`: everything a new session needs to continue it.

use crate::routing::{ModelChoice, ModelRouting, model_matches};
use crate::run::{Completion, Mode, Run, RunEventKind, RunStatus, Stage, TicketStatus};
use crate::{
    timestamps,
    usage::token_usage::{Usage, UsageStatus},
};
use std::collections::BTreeSet;

const UNKNOWN: &str = "unknown (not recorded)";

fn status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::Blocked => "blocked",
        RunStatus::Done => "done",
    }
}

fn ticket_mark(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::Pending => "pending    ",
        TicketStatus::InProgress => "in-progress",
        TicketStatus::Done => "done       ",
    }
}

/// Text from the record is shown on one line with control and direction-changing characters replaced,
/// so it can neither rewrite the terminal nor pass for another line of the report.
pub(crate) fn shown(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}' => '\u{FFFD}',
            c if c.is_control() => '\u{FFFD}',
            c => c,
        })
        .collect()
}

fn field(label: &str, value: Option<&str>) -> String {
    format!(
        "{label:<14}{}\n",
        value.map_or_else(|| UNKNOWN.to_string(), shown)
    )
}

/// The model each Stage is routed to; a Stage the selection does not map is not routed.
fn routing_section(routing: &ModelRouting, usage: Option<&Usage>, rework: Option<u32>) -> String {
    let effective_build = routing.effective(Stage::Build, rework);
    let mut out = format!("Model routing {}\n", routing.agent.name());
    for stage in Stage::ALL {
        let choice = routing.choice_for(stage);
        let line = match choice {
            Some(choice) => {
                let effort = choice
                    .effort
                    .as_deref()
                    .map_or(String::new(), |e| format!(", effort {}", shown(e)));
                let overridden = if routing.overrides.contains_key(&stage) {
                    " (override)"
                } else {
                    ""
                };
                format!("{}{effort}{overridden}", shown(&choice.model))
            }
            None => "not routed".to_string(),
        };
        out.push_str(&format!("  {:<6}  {line}\n", stage.name()));
        if let Some(complex) = routing.complex.get(&stage) {
            out.push_str(&format!(
                "          complex option {}{}\n",
                shown(&complex.model),
                complex
                    .effort
                    .as_deref()
                    .map_or(String::new(), |e| format!(", effort {}", shown(e)))
            ));
        }
        if stage == Stage::Finish
            && let Some(build) = effective_build.as_ref()
        {
            out.push_str(&format!(
                "          CI repair uses Build: {}{}\n",
                shown(&build.model),
                build
                    .effort
                    .as_deref()
                    .map_or(String::new(), |e| format!(", effort {}", shown(e)))
            ));
        }
        let ci = (stage == Stage::Finish)
            .then_some(effective_build.as_ref())
            .flatten();
        let requested = choice.or(ci);
        let prior = routing
            .prior_choices
            .get(&stage)
            .filter(|choices| !choices.is_empty());
        if requested.is_some() || prior.is_some() {
            let (observed, _) = observed_text(requested, stage, usage, routing, rework);
            out.push_str(&format!("          observed: {observed}\n"));
        }
        if let Some(prior) = prior {
            let choices: Vec<String> = prior
                .iter()
                .map(|choice| {
                    format!(
                        "{}{}",
                        shown(&choice.model),
                        choice
                            .effort
                            .as_deref()
                            .map_or(String::new(), |effort| format!(
                                ", effort {}",
                                shown(effort)
                            ))
                    )
                })
                .collect();
            out.push_str(&format!(
                "          prior assignments: {}\n",
                choices.join("; ")
            ));
        }
        if stage == Stage::Finish && ci.is_some() {
            out.push_str("          Finish usage aggregates PR text and CI repairs; task attribution is unavailable.\n");
        }
    }
    out
}

/// One routed stage for the dashboard: what was requested next to what the usage shows.
#[derive(Debug, serde::Serialize)]
pub(crate) struct RouteEntry {
    pub stage: Stage,
    pub model: String,
    pub effort: Option<String>,
    #[serde(rename = "override")]
    pub overridden: bool,
    /// The same text `jidoka run inspect` prints after `observed: `.
    pub observed: String,
    /// True exactly when `observed` holds a `MISMATCH:` verdict.
    pub mismatch: bool,
}

/// The routed stages in order; a stage the selection does not map is left out.
pub(crate) fn route_entries(
    routing: &ModelRouting,
    usage: Option<&Usage>,
    rework: Option<u32>,
) -> Vec<RouteEntry> {
    Stage::ALL
        .into_iter()
        .filter_map(|stage| {
            let choice = routing.effective(stage, rework)?;
            let (observed, mismatch) = observed_text(Some(&choice), stage, usage, routing, rework);
            Some(RouteEntry {
                stage,
                model: choice.model.clone(),
                effort: choice.effort.clone(),
                overridden: routing.overrides.contains_key(&stage),
                observed,
                mismatch,
            })
        })
        .collect()
}

/// What the run's usage shows for one model in one stage.
struct Seen<'a> {
    model: &'a str,
    efforts: BTreeSet<&'a str>,
    subagents: bool,
}

/// The models the run's usage shows for `stage`, next to whether the request is met. The request is an
/// alias or an id and the log has an id, so both are shown and only a missing request counts. When an
/// effort was requested, it is compared with the efforts the logs recorded for the models that match.
fn observed_text(
    requested: Option<&ModelChoice>,
    stage: Stage,
    usage: Option<&Usage>,
    routing: &ModelRouting,
    rework: Option<u32>,
) -> (String, bool) {
    let Some(usage) = usage else {
        return (
            "not tracked: usage has not been collected for this run yet".into(),
            false,
        );
    };
    if usage.status == UsageStatus::NotTracked {
        let reason = usage.reason.as_deref().unwrap_or("no reason recorded");
        return (format!("not tracked: {}", shown(reason)), false);
    }
    // Models in the order they first appear.
    let mut seen: Vec<Seen> = Vec::new();
    for row in usage.rows.iter().filter(|row| row.stage == stage) {
        let subagents = !row.subagents.is_empty();
        let efforts = row.efforts.iter().map(String::as_str);
        match seen.iter_mut().find(|s| s.model == row.model) {
            Some(known) => {
                known.subagents |= subagents;
                known.efforts.extend(efforts);
            }
            None => seen.push(Seen {
                model: &row.model,
                efforts: efforts.collect(),
                subagents,
            }),
        }
    }
    let partial = usage.status == UsageStatus::Partial;
    if seen.is_empty() {
        let why = match (usage.no_data.contains(&stage), partial) {
            (true, false) => " (no session log has entries for it)",
            (true, true) => " (no session log has entries for it; usage is partial)",
            (false, true) => " (usage is partial)",
            (false, false) => "",
        };
        return (format!("none recorded for this stage{why}"), false);
    }
    let list: Vec<String> = seen
        .iter()
        .map(|s| {
            let mut notes = Vec::new();
            if !s.efforts.is_empty() {
                notes.push(format!(
                    "effort {}",
                    join_efforts(s.efforts.iter().copied())
                ));
            }
            if s.subagents {
                notes.push("+subagents".to_string());
            }
            if notes.is_empty() {
                shown(s.model)
            } else {
                format!("{} ({})", shown(s.model), notes.join(", "))
            }
        })
        .collect();
    let (verdict, mismatch) = if let Some(requested) = requested {
        if (stage == Stage::Finish && routing.choice_for(Stage::Build).is_some())
            || routing
                .prior_choices
                .get(&stage)
                .is_some_and(|choices| !choices.is_empty())
            || (stage == Stage::Build && rework.is_some())
        {
            aggregate_verdict(routing, stage, &seen, partial)
        } else {
            verdict(requested, &seen, partial)
        }
    } else {
        (
            "no current route; aggregate usage cannot establish prior compliance".into(),
            false,
        )
    };
    (format!("{}; {verdict}", list.join(", ")), mismatch)
}

/// Aggregate usage cannot establish which assignment or effort each subagent used.
fn aggregate_verdict(
    routing: &ModelRouting,
    stage: Stage,
    seen: &[Seen],
    partial: bool,
) -> (String, bool) {
    if !seen.iter().any(|s| s.subagents) {
        return if partial {
            ("kept on the coordinator (usage is partial)".into(), false)
        } else {
            ("kept on the coordinator".into(), false)
        };
    }
    let unexpected: Vec<&str> = seen
        .iter()
        .filter(|s| s.subagents && !routing.allows_model(stage, s.model))
        .map(|s| s.model)
        .collect();
    if unexpected.is_empty() {
        if partial {
            ("allowed models observed (usage is partial)".into(), false)
        } else {
            (
                "allowed models observed; assignment and effort attribution unavailable".into(),
                false,
            )
        }
    } else if partial {
        (
            format!(
                "unrelated model {} observed (usage is partial)",
                unexpected
                    .into_iter()
                    .map(shown)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            false,
        )
    } else {
        (
            format!(
                "MISMATCH: unrelated subagent model {} observed",
                unexpected
                    .into_iter()
                    .map(shown)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            true,
        )
    }
}

/// Whether the request is met by what was observed. Only subagents are routed, so their models are judged;
/// a stage with only coordinator usage is neutral.
fn verdict(requested: &ModelChoice, seen: &[Seen], partial: bool) -> (String, bool) {
    let model = shown(&requested.model);
    if !seen.iter().any(|s| s.subagents) {
        let note = if partial { "; usage is partial" } else { "" };
        let text = if seen
            .iter()
            .any(|s| model_matches(&requested.model, s.model))
        {
            format!("matches the request (kept on the coordinator{note})")
        } else if partial {
            "kept on the coordinator (usage is partial)".to_string()
        } else {
            "kept on the coordinator".to_string()
        };
        return (text, false);
    }
    let matching: Vec<&Seen> = seen
        .iter()
        .filter(|s| s.subagents && model_matches(&requested.model, s.model))
        .collect();
    if matching.is_empty() {
        return if partial {
            (
                format!("{model} not observed (usage is partial, so it may be missing)"),
                false,
            )
        } else {
            (
                format!("MISMATCH: {model} was requested and never observed"),
                true,
            )
        };
    }
    let mut notes = Vec::new();
    if let Some(want) = &requested.effort {
        let efforts: BTreeSet<&str> = matching
            .iter()
            .flat_map(|s| s.efforts.iter().copied())
            .collect();
        if efforts.is_empty() {
            notes.push("effort not recorded");
        } else if !efforts.iter().any(|e| e.eq_ignore_ascii_case(want)) {
            let (want, got) = (shown(want), join_efforts(efforts.iter().copied()));
            return if partial {
                (
                    format!(
                        "effort {want} not observed, only {got} (usage is partial, so it may be missing)"
                    ),
                    false,
                )
            } else {
                (
                    format!("MISMATCH: effort {want} was requested, observed {got}"),
                    true,
                )
            };
        }
    }
    if partial {
        notes.push("usage is partial");
    }
    let text = if notes.is_empty() {
        "matches the request".to_string()
    } else {
        format!("matches the request ({})", notes.join("; "))
    };
    (text, false)
}

/// Efforts as one piece of text, such as `high/medium`.
fn join_efforts<'a>(efforts: impl IntoIterator<Item = &'a str>) -> String {
    efforts.into_iter().map(shown).collect::<Vec<_>>().join("/")
}

/// The change group a run is in, shown only for a grouped run.
pub(crate) fn group_line(group: &str) -> String {
    field("Group", Some(group))
}

/// Where this run's plan is saved, as the last line of the report.
pub(crate) fn plan_line(plan: &str) -> String {
    field("Plan", Some(plan))
}

fn render_tickets(run: &Run, out: &mut String) {
    let done = run
        .tickets
        .iter()
        .filter(|t| t.status == TicketStatus::Done)
        .count();
    if run.tickets.is_empty() {
        out.push_str(if run.unticketed_attempt.is_some() {
            "Tickets       unticketed work\n"
        } else {
            "Tickets       none recorded\n"
        });
    } else {
        out.push_str(&format!(
            "Tickets       {done} of {} done\n",
            run.tickets.len()
        ));
        for ticket in &run.tickets {
            out.push_str(&format!(
                "  [{}] {} {}\n",
                ticket_mark(ticket.status),
                shown(&ticket.reference),
                shown(&ticket.title)
            ));
            if let Some(result) = crate::run_gate::latest_ticket_verification(run, ticket) {
                if let Some(commit) = &result.commit {
                    out.push_str(&format!("    Verification recorded at {}\n", shown(commit)));
                } else if !result.passed {
                    out.push_str("    Verification failed\n");
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "formats every section of the run report as one long string builder"
)]
pub(crate) fn render(run: &Run) -> String {
    let mut out = format!("Run {}: {}\n", run.id, shown(&run.title));
    out.push_str(&format!(
        "{:<14}{} ({}){}\n",
        "Stage",
        run.stage.name(),
        status_name(run.status),
        run.rework
            .map_or(String::new(), |n| format!(", rework {n}"))
    ));
    if let Some(reason) = &run.blocking_reason {
        out.push_str(&field("Status reason", Some(reason)));
    }
    if let Some(activity) = &run.activity {
        out.push_str(&field("Activity", Some(activity)));
    }
    out.push_str(&field("Mode", run.mode.map(Mode::name)));
    out.push_str(&field("Repository", run.repository.as_deref()));
    out.push_str(&field("Started in", Some(&run.project)));
    out.push_str(&field("Worktree", run.worktree.as_deref()));
    out.push_str(&field("Branch", run.branch.as_deref()));
    out.push_str(&field("Base commit", run.base_commit.as_deref()));
    out.push_str(&field("Spec", run.spec.as_deref()));
    out.push_str(&field("Pull request", run.pr.as_deref()));
    if let Some(status) = run
        .pr_status
        .as_ref()
        .filter(|status| Some(&status.url) == run.pr.as_ref())
    {
        out.push_str(&field("PR state", Some(status.state.label())));
    }
    let completion = match &run.completion {
        Some(Completion::Ci { head, outcome }) => Some(format!("{} on {head}", outcome.label())),
        Some(Completion::Merged { url }) => Some(format!("PR merge observed ({url})")),
        Some(Completion::Override { reason }) => Some(format!("Override ({reason})")),
        None if run.status == RunStatus::Done => Some(UNKNOWN.into()),
        None => None,
    };
    if let Some(completion) = completion {
        out.push_str(&field("Completion", Some(&completion)));
    }
    render_tickets(run, &mut out);
    if let Some(routing) = &run.model_routing {
        out.push_str(&routing_section(routing, run.usage.as_ref(), run.rework));
        if run.rework.is_some()
            && let Some(choice) = routing.effective(Stage::Build, run.rework)
        {
            out.push_str(&format!("  effective Build for rework and CI: {}{} (known lower effort raised to high; unknown or stronger preserved)\n", shown(&choice.model), choice.effort.as_deref().map_or(String::new(), |e| format!(", effort {}", shown(e)))));
        }
    }
    for report in &run.run_reports {
        if report.text.starts_with(crate::routing::WARNING_PREFIX) {
            out.push_str(&format!("  {}\n", shown(&report.text)));
        }
    }
    if !run.findings.is_empty() {
        out.push_str("Findings\n");
        for finding in &run.findings {
            let state = format!("{:?}", finding.state).to_lowercase();
            let reason = finding
                .reason
                .as_deref()
                .map_or_else(String::new, |reason| format!(" ({})", shown(reason)));
            let fix = finding
                .fix_commit
                .as_deref()
                .map_or_else(String::new, |commit| {
                    format!(", fixed in {}", shown(commit))
                });
            let rework = finding
                .rework
                .map_or_else(String::new, |n| format!(", sent back to rework {n}"));
            out.push_str(&format!(
                "  {} {state}{reason}  {}: {}{}\n    reviewed {}{rework}{fix}\n",
                finding.id,
                shown(&finding.check),
                shown(&finding.text),
                finding
                    .location
                    .as_deref()
                    .map_or_else(String::new, |l| format!(" [{}]", shown(l))),
                shown(&finding.reviewed_commit),
            ));
        }
    }
    if let Some(ticket) = run.active_ticket() {
        out.push_str(&field("Current ticket", Some(&ticket.reference)));
    }
    if run.summaries.is_empty() {
        out.push_str("Summaries     none recorded\n");
    } else {
        out.push_str("Summaries\n");
        for (stage, text) in &run.summaries {
            out.push_str(&format!("  {}: {}\n", stage.name(), shown(text)));
        }
    }
    if !run.events.is_empty() {
        out.push_str("Events\n");
        for event in &run.events {
            let kind = match event.kind {
                RunEventKind::Blocked => "blocked",
                RunEventKind::Resumed => "resumed",
            };
            let reason = event
                .reason
                .as_deref()
                .map_or_else(String::new, |reason| format!(": {}", shown(reason)));
            out.push_str(&format!(
                "  {}  {kind}  {}{reason}\n",
                timestamps::format(event.at),
                event.stage.name()
            ));
        }
    }
    if let Some(usage) = &run.usage {
        if !usage.pauses.is_empty() || !usage.idle.is_empty() {
            out.push_str("Pauses\n");
        }
        for pause in &usage.pauses {
            let resumed = pause.resumed_at.map_or_else(
                || "not resumed".to_string(),
                |at| format!("resumed after {}", duration(at.saturating_sub(pause.at))),
            );
            out.push_str(&format!(
                "  {}  {}, {resumed}\n",
                timestamps::format(pause.at),
                pause.kind.reason().to_lowercase()
            ));
        }
        for gap in &usage.idle {
            out.push_str(&format!(
                "  {}  idle {}\n",
                timestamps::format(gap.from),
                duration(gap.to.saturating_sub(gap.from))
            ));
        }
    }
    out
}

/// Milliseconds in a minute.
const MS_PER_MINUTE: u64 = 60_000;
/// Minutes in an hour.
const MINUTES_PER_HOUR: u64 = 60;

/// A span of milliseconds as minutes, or hours and minutes.
fn duration(ms: u64) -> String {
    let minutes = ms / MS_PER_MINUTE;
    if minutes < MINUTES_PER_HOUR {
        format!("{minutes} min")
    } else {
        format!(
            "{} h {} min",
            minutes / MINUTES_PER_HOUR,
            minutes % MINUTES_PER_HOUR
        )
    }
}

/// `jidoka run inspect <id> [--json]`: the run's recorded context, its change group and its plan path.
pub(crate) struct InspectCommand;

/// What `run inspect` shows: the run, its plan path and its change group.
pub(crate) struct Inspected {
    run: Run,
    plan: String,
    group: Option<String>,
}

impl crate::cli::Command for InspectCommand {
    /// The run id, and whether to print JSON.
    type Options = (String, bool);
    type Outcome = Inspected;

    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn std::error::Error>> {
        let (id, options) = crate::cli::split_first(words);
        let json = crate::cli::parse_json_flag(options)?;
        Ok((id.to_owned(), json))
    }

    fn run((id, _): &Self::Options) -> Result<Inspected, Box<dyn std::error::Error>> {
        let run = crate::client::get_run(id)?;
        let plan = crate::client::plan_path(id)?.to_string_lossy().into_owned();
        let group = crate::client::group_of_run(id)?;
        Ok(Inspected { run, plan, group })
    }

    fn render(
        &(_, json): &Self::Options,
        inspected: Inspected,
    ) -> Result<crate::cli::Output, Box<dyn std::error::Error>> {
        let Inspected { run, plan, group } = inspected;
        if json {
            let mut shown = serde_json::to_value(&run)?;
            shown["plan"] = plan.into();
            if let Some(group) = &group {
                shown["group"] = group.as_str().into();
            }
            return Ok(crate::cli::Output::json(&shown)?);
        }
        let mut text = render(&run);
        if let Some(group) = &group {
            text.push_str(&group_line(group));
        }
        text.push_str(&plan_line(&plan));
        Ok(crate::cli::Output::text(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_summary(summary: &str) -> Run {
        serde_json::from_value(serde_json::json!({
            "id": "1", "title": "t", "gates": [], "project": "/p", "stage": "build", "status": "running",
            "summaries": {"define": summary}
        }))
        .unwrap()
    }

    #[test]
    fn pauses_and_idle_gaps_are_listed() {
        let mut run = run_with_summary("x");
        run.usage = Some(
            serde_json::from_value(serde_json::json!({
                "status": "tracked", "collected_at": 1,
                "pauses": [
                    {"kind": "usage_limit", "at": 1_793_872_800_000_u64, "resumed_at": 1_793_883_600_000_u64},
                    {"kind": "compaction", "at": 1_793_890_000_000_u64}
                ],
                "idle": [{"from": 1_793_872_800_000_u64, "to": 1_793_873_820_000_u64}]
            }))
            .unwrap(),
        );
        let text = render(&run);
        assert!(text.contains("Pauses\n"), "{text}");
        assert!(
            text.contains("usage limit reached, resumed after 3 h 0 min"),
            "{text}"
        );
        assert!(
            text.contains("context compaction in progress, not resumed"),
            "{text}"
        );
        assert!(text.contains("idle 17 min"), "{text}");
        assert!(!render(&run_with_summary("x")).contains("Pauses"));
    }

    fn routed_run() -> Run {
        serde_json::from_value(serde_json::json!({
            "id": "1", "title": "t", "gates": [], "project": "/p", "stage": "build", "status": "running",
            "model_routing": {
                "agent": "claude",
                "stages": {
                    "build": {"model": "sonnet"},
                    "check": {"model": "opus", "effort": "high"}
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn model_routing_is_shown_per_stage_only_when_a_selection_exists() {
        let text = render(&routed_run());
        let lines: Vec<_> = text
            .lines()
            .skip_while(|line| !line.starts_with("Model routing"))
            .take(5)
            .collect();
        assert_eq!(
            lines,
            [
                "Model routing claude",
                "  define  not routed",
                "  build   sonnet",
                "          observed: not tracked: usage has not been collected for this run yet",
                "  check   opus, effort high",
            ],
            "{text}"
        );
        assert!(text.contains("  finish  not routed\n"), "{text}");
        assert!(!text.contains("Effort observed"), "{text}");
        assert!(!render(&run_with_summary("s")).contains("Model routing"));
        // A Copilot selection is shown like any other agent's.
        let mut copilot = routed_run();
        copilot.model_routing.as_mut().unwrap().agent = crate::agent::Agent::Copilot;
        assert!(render(&copilot).contains("Model routing copilot\n"));
        // The machine-readable form is the whole run, so skills can read the selection from it.
        let json = serde_json::to_value(routed_run()).unwrap();
        assert_eq!(json["model_routing"]["stages"]["check"]["model"], "opus");
    }

    /// The routed run, with usage rows `(stage, model, used by subagents)` and a coverage status.
    fn with_usage(status: &str, rows: &[(&str, &str, bool)]) -> Run {
        let mut run = routed_run();
        let rows: Vec<_> = rows
            .iter()
            .map(|(stage, model, subagents)| {
                let mut row =
                    serde_json::json!({"stage": stage, "model": model, "input": 10, "output": 5});
                if *subagents {
                    row["subagents"] = serde_json::json!({"input": 4, "output": 2});
                }
                row
            })
            .collect();
        run.usage = Some(
            serde_json::from_value(serde_json::json!({
                "status": status, "collected_at": 1, "rows": rows,
                "reason": if status == "not-tracked" { Some("no session logs were found") } else { None },
            }))
            .unwrap(),
        );
        run
    }

    fn observed(text: &str, task: &str) -> String {
        let mut lines = text
            .lines()
            .skip_while(|l| !l.starts_with(&format!("  {task:<6}")));
        lines.next();
        lines.next().unwrap_or_default().trim().to_string()
    }

    /// Records the efforts the logs showed for the rows of `model` in `stage`.
    fn set_efforts(run: &mut Run, stage: Stage, model: &str, efforts: &[&str]) {
        let row = run
            .usage
            .as_mut()
            .unwrap()
            .rows
            .iter_mut()
            .find(|row| row.stage == stage && row.model == model)
            .unwrap();
        row.efforts = efforts.iter().map(ToString::to_string).collect();
    }

    #[test]
    fn finish_usage_accepts_both_routes_and_reports_unrelated_models() {
        let make = |status, rows: &[(&str, &str, bool)]| {
            let mut run = with_usage(status, rows);
            run.model_routing.as_mut().unwrap().stages.insert(
                Stage::Finish,
                ModelChoice {
                    model: "haiku".into(),
                    effort: Some("low".into()),
                },
            );
            render(&run)
        };
        let text = make("tracked", &[("finish", "claude-sonnet-5-5", true)]);
        assert!(
            text.contains("claude-sonnet-5-5 (+subagents); allowed models observed"),
            "{text}"
        );
        let text = make("tracked", &[("finish", "claude-opus-5-5", true)]);
        assert!(
            text.contains("MISMATCH: unrelated subagent model"),
            "{text}"
        );
        let text = make("partial", &[("finish", "claude-opus-5-5", true)]);
        assert!(
            text.contains("unrelated model claude-opus-5-5 observed (usage is partial)"),
            "{text}"
        );
        let text = make("tracked", &[("finish", "claude-opus-5-5", false)]);
        assert!(text.contains("kept on the coordinator"), "{text}");
        let mut only_build = with_usage("tracked", &[("finish", "claude-sonnet-5-5", true)]);
        only_build.rework = Some(1);
        assert!(render(&only_build).contains("allowed models observed"));
    }

    #[test]
    fn the_observed_effort_is_shown_and_met_when_the_matching_model_ran_at_the_requested_one() {
        let mut run = with_usage(
            "tracked",
            &[
                ("build", "claude-sonnet-5-5", false),
                ("check", "claude-sonnet-5-5", false),
                ("check", "claude-opus-5-5", true),
            ],
        );
        set_efforts(&mut run, Stage::Build, "claude-sonnet-5-5", &["medium"]);
        set_efforts(&mut run, Stage::Check, "claude-sonnet-5-5", &["medium"]);
        set_efforts(&mut run, Stage::Check, "claude-opus-5-5", &["high"]);
        let text = render(&run);
        // The coordinator's own effort is shown but does not count: only the matching model is compared.
        assert_eq!(
            observed(&text, "check"),
            "observed: claude-sonnet-5-5 (effort medium), claude-opus-5-5 (effort high, +subagents); matches the request"
        );
        // No effort was requested for build, so the observed one is only shown.
        assert_eq!(
            observed(&text, "build"),
            "observed: claude-sonnet-5-5 (effort medium); matches the request (kept on the coordinator)"
        );
    }

    #[test]
    fn an_effort_other_than_the_requested_one_is_a_mismatch_unless_usage_is_partial() {
        let mut run = with_usage("tracked", &[("check", "claude-opus-5-5", true)]);
        set_efforts(&mut run, Stage::Check, "claude-opus-5-5", &["medium"]);
        assert_eq!(
            observed(&render(&run), "check"),
            "observed: claude-opus-5-5 (effort medium, +subagents); MISMATCH: effort high was requested, observed medium"
        );
        let mut partial = with_usage("partial", &[("check", "claude-opus-5-5", true)]);
        set_efforts(&mut partial, Stage::Check, "claude-opus-5-5", &["medium"]);
        assert_eq!(
            observed(&render(&partial), "check"),
            "observed: claude-opus-5-5 (effort medium, +subagents); effort high not observed, only medium (usage is partial, so it may be missing)"
        );
    }

    #[test]
    fn route_entries_follow_inspect_and_flag_only_mismatches() {
        let mut run = with_usage(
            "tracked",
            &[
                ("build", "claude-sonnet-5-5", false),
                ("check", "claude-opus-5-5", true),
            ],
        );
        set_efforts(&mut run, Stage::Check, "claude-opus-5-5", &["medium"]);
        let routing = run.model_routing.as_ref().unwrap();
        let entries = route_entries(routing, run.usage.as_ref(), run.rework);
        let stages: Vec<_> = entries.iter().map(|e| e.stage.name()).collect();
        assert_eq!(stages, ["build", "check"]);
        assert!(!entries[0].mismatch && !entries[0].overridden);
        assert_eq!(entries[1].effort.as_deref(), Some("high"));
        assert!(entries[1].mismatch);
        let text = render(&run);
        for entry in &entries {
            assert!(
                text.contains(&format!("observed: {}\n", entry.observed)),
                "{text}"
            );
        }
        let json = serde_json::to_value(&entries[1]).unwrap();
        assert_eq!(json["stage"], "check");
        assert_eq!(json["override"], false);
        assert!(
            route_entries(routing, None, None)
                .iter()
                .all(|e| !e.mismatch)
        );
    }

    #[test]
    fn route_entries_use_effective_build_and_allow_aggregate_history_and_finish_repairs() {
        let mut run = with_usage(
            "tracked",
            &[
                ("build", "claude-sonnet-5-5", true),
                ("build", "claude-opus-5-5", true),
                ("finish", "claude-sonnet-5-5", true),
            ],
        );
        run.rework = Some(1);
        let routing = run.model_routing.as_mut().unwrap();
        routing.stages.get_mut(&Stage::Build).unwrap().effort = Some("medium".into());
        routing.stages.insert(
            Stage::Finish,
            ModelChoice {
                model: "haiku".into(),
                effort: None,
            },
        );
        routing.prior_choices.insert(
            Stage::Build,
            vec![ModelChoice {
                model: "opus".into(),
                effort: None,
            }],
        );
        set_efforts(&mut run, Stage::Build, "claude-sonnet-5-5", &["medium"]);
        let entries = route_entries(
            run.model_routing.as_ref().unwrap(),
            run.usage.as_ref(),
            run.rework,
        );
        let build = entries.iter().find(|e| e.stage == Stage::Build).unwrap();
        assert_eq!(build.effort.as_deref(), Some("high"));
        assert!(!build.mismatch && build.observed.contains("allowed models observed"));
        let finish = entries.iter().find(|e| e.stage == Stage::Finish).unwrap();
        assert_eq!(finish.model, "haiku");
        assert!(!finish.mismatch && finish.observed.contains("allowed models observed"));
        run.usage.as_mut().unwrap().rows[0].model = "unrelated".into();
        let entries = route_entries(
            run.model_routing.as_ref().unwrap(),
            run.usage.as_ref(),
            run.rework,
        );
        assert!(
            entries
                .iter()
                .find(|e| e.stage == Stage::Build)
                .unwrap()
                .mismatch
        );
    }

    #[test]
    fn a_matching_run_shows_the_alias_and_the_observed_id_side_by_side() {
        let text = render(&with_usage(
            "tracked",
            &[
                ("build", "claude-sonnet-5-5", false),
                ("check", "claude-sonnet-5-5", false),
                ("check", "claude-opus-5-5", true),
            ],
        ));
        assert_eq!(
            observed(&text, "build"),
            "observed: claude-sonnet-5-5; matches the request (kept on the coordinator)"
        );
        // The coordinator ran on Sonnet, the review agents on Opus: the request is met.
        assert_eq!(
            observed(&text, "check"),
            "observed: claude-sonnet-5-5, claude-opus-5-5 (+subagents); matches the request (effort not recorded)"
        );
        assert!(!text.contains("MISMATCH"), "{text}");
        assert!(!text.contains("Effort observed"), "{text}");
        assert!(text.contains("  check   opus, effort high\n"), "{text}");
    }

    #[test]
    fn subagents_on_another_model_are_a_mismatch_even_when_the_coordinator_matches() {
        let text = render(&with_usage(
            "tracked",
            &[
                ("check", "claude-opus-5-5", false),
                ("check", "claude-sonnet-5-5", true),
            ],
        ));
        assert_eq!(
            observed(&text, "check"),
            "observed: claude-opus-5-5, claude-sonnet-5-5 (+subagents); MISMATCH: opus was requested and never observed"
        );
    }

    #[test]
    fn a_coordinator_only_stage_is_neutral() {
        let text = render(&with_usage(
            "tracked",
            &[
                ("build", "claude-sonnet-5-5", false),
                ("check", "claude-sonnet-5-5", false),
            ],
        ));
        assert_eq!(
            observed(&text, "check"),
            "observed: claude-sonnet-5-5; kept on the coordinator"
        );
        assert!(!text.contains("MISMATCH"), "{text}");
        let mut high = with_usage("tracked", &[("check", "claude-opus-5-5", false)]);
        set_efforts(&mut high, Stage::Check, "claude-opus-5-5", &["low"]);
        let text = render(&high);
        assert!(!text.contains("MISMATCH"), "{text}");
        assert!(
            text.contains("matches the request (kept on the coordinator)"),
            "{text}"
        );
    }

    #[test]
    fn rework_usage_counts_toward_the_build_task() {
        let mut run = with_usage("tracked", &[]);
        let row: crate::usage::token_usage::UsageRow = serde_json::from_value(serde_json::json!(
            {"stage": "build", "model": "claude-sonnet-5-5", "rework": true, "input": 1, "output": 1}
        ))
        .unwrap();
        run.usage.as_mut().unwrap().rows.push(row);
        assert!(observed(&render(&run), "build").contains("matches the request"));
    }

    #[test]
    fn rework_and_prior_choices_report_aggregate_usage_without_hiding_unrelated_models() {
        for efforts in [&["high"][..], &["medium", "high"][..]] {
            let mut run = with_usage("tracked", &[("build", "claude-sonnet-5-5", true)]);
            run.rework = Some(1);
            run.model_routing
                .as_mut()
                .unwrap()
                .stages
                .get_mut(&Stage::Build)
                .unwrap()
                .effort = Some("medium".into());
            set_efforts(&mut run, Stage::Build, "claude-sonnet-5-5", &[efforts[0]]);
            if efforts.len() == 2 {
                let mut rework_row = run.usage.as_ref().unwrap().rows[0].clone();
                rework_row.rework = true;
                rework_row.efforts = [efforts[1].into()].into_iter().collect();
                run.usage.as_mut().unwrap().rows.push(rework_row);
            } else {
                run.usage.as_mut().unwrap().rows[0].rework = true;
            }
            let text = render(&run);
            assert!(
                text.contains("effective Build for rework and CI: sonnet, effort high"),
                "{text}"
            );
            assert!(
                text.contains("CI repair uses Build: sonnet, effort high"),
                "{text}"
            );
            assert!(
                observed(&text, "build").contains("assignment and effort attribution unavailable"),
                "{text}"
            );
            assert!(!observed(&text, "build").contains("MISMATCH"), "{text}");
        }
        let mut run = with_usage("tracked", &[("build", "claude-sonnet-5-5", true)]);
        let routing = run.model_routing.as_mut().unwrap();
        routing
            .prior_choices
            .insert(Stage::Build, vec![routing.stages[&Stage::Build].clone()]);
        routing.overrides.insert(
            Stage::Build,
            ModelChoice {
                model: "opus".into(),
                effort: Some("high".into()),
            },
        );
        assert!(observed(&render(&run), "build").contains("allowed models observed"));
        run.usage.as_mut().unwrap().rows[0].model = "unrelated".into();
        assert!(observed(&render(&run), "build").contains("MISMATCH: unrelated subagent model"));
    }

    #[test]
    fn cleared_route_keeps_prior_assignments_without_judging_aggregate_usage() {
        let mut run = with_usage("tracked", &[("build", "unrelated", true)]);
        let routing = run.model_routing.as_mut().unwrap();
        let prior = routing.stages.remove(&Stage::Build).unwrap();
        routing.prior_choices.insert(Stage::Build, vec![prior]);

        let text = render(&run);
        assert!(text.contains("  build   not routed"), "{text}");
        assert!(text.contains("prior assignments: sonnet"), "{text}");
        let usage = observed(&text, "build");
        assert!(usage.contains("unrelated (+subagents)"), "{text}");
        assert!(
            usage.contains("no current route; aggregate usage cannot establish prior compliance"),
            "{text}"
        );
        assert!(!usage.contains("MISMATCH"), "{text}");
        assert!(!usage.contains("matches the request"), "{text}");
        assert!(!usage.contains("allowed models observed"), "{text}");
    }

    #[test]
    fn blocked_and_resumed_events_are_listed_with_time_stage_and_reason() {
        let mut run = run_with_summary("x");
        assert!(!render(&run).contains("Events"));
        run.events = serde_json::from_value(serde_json::json!([
            {"at": 1_762_636_800_000_u64, "kind": "blocked", "stage": "build", "reason": "Needs\nan answer"},
            {"at": 1_762_640_400_000_u64, "kind": "resumed", "stage": "build"}
        ]))
        .unwrap();
        let text = render(&run);
        assert!(
            text.contains("Events\n  2025-11-08 21:20 UTC  blocked  build: Needs an answer\n  2025-11-08 22:20 UTC  resumed  build\n"),
            "{text}"
        );
    }

    #[test]
    fn stored_text_cannot_pass_for_another_line_or_rewrite_the_terminal() {
        let text = render(&run_with_summary(
            "ok\nPull request  https://evil.test\u{1b}[2J\u{202E}x",
        ));
        assert!(
            !text.contains('\u{1b}') && !text.contains('\u{202E}'),
            "{text:?}"
        );
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("Pull request"))
                .count(),
            1,
            "{text}"
        );
        assert!(
            text.contains("define: ok Pull request  https://evil.test"),
            "{text}"
        );
    }
}
