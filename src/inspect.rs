//! The readable form of a run, for `jidoka run inspect`: everything a new session needs to continue it.

use crate::routing::{ModelChoice, ModelRouting, model_matches};
use crate::run::{Mode, Run, RunEventKind, RunStatus, Stage, TicketStatus};
use crate::usage::{Usage, UsageStatus, format_timestamp};
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
fn routing_section(routing: &ModelRouting, usage: Option<&Usage>) -> String {
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
        if let Some(requested) = choice {
            out.push_str(&format!(
                "          {}\n",
                observed_line(requested, stage, usage)
            ));
        }
    }
    out
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
fn observed_line(requested: &ModelChoice, stage: Stage, usage: Option<&Usage>) -> String {
    let Some(usage) = usage else {
        return "observed: not tracked: usage has not been collected for this run yet".into();
    };
    if usage.status == UsageStatus::NotTracked {
        let reason = usage.reason.as_deref().unwrap_or("no reason recorded");
        return format!("observed: not tracked: {}", shown(reason));
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
        return format!("observed: none recorded for this stage{why}");
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
    format!(
        "observed: {}; {}",
        list.join(", "),
        verdict(requested, &seen, partial)
    )
}

/// Whether the request is met by what was observed. Only subagents are routed, so their models are judged;
/// a stage with only coordinator usage is neutral.
fn verdict(requested: &ModelChoice, seen: &[Seen], partial: bool) -> String {
    let model = shown(&requested.model);
    if !seen.iter().any(|s| s.subagents) {
        let note = if partial { "; usage is partial" } else { "" };
        return if seen
            .iter()
            .any(|s| model_matches(&requested.model, s.model))
        {
            format!("matches the request (kept on the coordinator{note})")
        } else if partial {
            "kept on the coordinator (usage is partial)".to_string()
        } else {
            "kept on the coordinator".to_string()
        };
    }
    let matching: Vec<&Seen> = seen
        .iter()
        .filter(|s| s.subagents && model_matches(&requested.model, s.model))
        .collect();
    if matching.is_empty() {
        return if partial {
            format!("{model} not observed (usage is partial, so it may be missing)")
        } else {
            format!("MISMATCH: {model} was requested and never observed")
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
                format!(
                    "effort {want} not observed, only {got} (usage is partial, so it may be missing)"
                )
            } else {
                format!("MISMATCH: effort {want} was requested, observed {got}")
            };
        }
    }
    if partial {
        notes.push("usage is partial");
    }
    if notes.is_empty() {
        "matches the request".to_string()
    } else {
        format!("matches the request ({})", notes.join("; "))
    }
}

/// Efforts as one piece of text, such as `high/medium`.
fn join_efforts<'a>(efforts: impl IntoIterator<Item = &'a str>) -> String {
    efforts.into_iter().map(shown).collect::<Vec<_>>().join("/")
}

/// Where this run's plan is saved, as the last line of the report.
pub(crate) fn plan_line(plan: &str) -> String {
    field("Plan", Some(plan))
}

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
    let done = run
        .tickets
        .iter()
        .filter(|t| t.status == TicketStatus::Done)
        .count();
    if run.tickets.is_empty() {
        out.push_str("Tickets       none recorded\n");
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
        }
    }
    if let Some(routing) = &run.model_routing {
        out.push_str(&routing_section(routing, run.usage.as_ref()));
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
    if let Some(ticket) = &run.current_ticket {
        out.push_str(&field("Current ticket", Some(ticket)));
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
                format_timestamp(event.at),
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
                format_timestamp(pause.at),
                pause.kind.reason().to_lowercase()
            ));
        }
        for gap in &usage.idle {
            out.push_str(&format!(
                "  {}  idle {}\n",
                format_timestamp(gap.from),
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
        copilot.model_routing.as_mut().unwrap().agent = crate::routing::RoutingAgent::Copilot;
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
        let row: crate::usage::UsageRow = serde_json::from_value(serde_json::json!(
            {"stage": "build", "model": "claude-sonnet-5-5", "rework": true, "input": 1, "output": 1}
        ))
        .unwrap();
        run.usage.as_mut().unwrap().rows.push(row);
        assert!(observed(&render(&run), "build").contains("matches the request"));
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
