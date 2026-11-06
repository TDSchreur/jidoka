//! The readable form of a run, for `factory run inspect`: everything a new session needs to continue it.

use crate::routing::{model_matches, ModelRouting, Task};
use crate::run::{Mode, Run, RunStatus, Stage, TicketStatus};
use crate::usage::{Usage, UsageStatus};

const UNKNOWN: &str = "unknown (not recorded)";

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Define => "define",
        Stage::Build => "build",
        Stage::Check => "check",
        Stage::Finish => "finish",
    }
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Full => "factory-full",
        Mode::Balanced => "factory-balanced",
        Mode::Fast => "factory-fast",
    }
}

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

/// The model each task is routed to; a task the selection does not map is not routed.
fn routing_section(routing: &ModelRouting, usage: Option<&Usage>) -> String {
    let mut out = format!("Model routing {}\n", routing.agent.name());
    for task in Task::ALL {
        let line = match routing.role_for(task) {
            Some(role) => {
                let choice = routing.roles.get(&role);
                let model = choice.map_or_else(|| UNKNOWN.to_string(), |c| shown(&c.model));
                let effort = choice
                    .and_then(|c| c.effort.as_deref())
                    .map_or(String::new(), |e| format!(", effort {}", shown(e)));
                let overridden = if routing.overrides.contains_key(&task) {
                    " (override)"
                } else {
                    ""
                };
                format!("{:<7} {model}{effort}{overridden}", role.name())
            }
            None => "not routed".to_string(),
        };
        out.push_str(&format!("  {:<6}  {line}\n", task.name()));
        if let Some(requested) = routing
            .role_for(task)
            .and_then(|role| routing.roles.get(&role))
        {
            out.push_str(&format!(
                "          {}\n",
                observed_line(&requested.model, stage_of(task), usage)
            ));
        }
    }
    out.push_str("  Effort observed: unknown (the agents' session logs do not record it)\n");
    out
}

/// A task runs in the stage of the same name.
fn stage_of(task: Task) -> Stage {
    match task {
        Task::Build => Stage::Build,
        Task::Check => Stage::Check,
        Task::Finish => Stage::Finish,
    }
}

/// The models the run's usage shows for `stage`, next to whether the requested one is among them. The
/// request is an alias or an id and the log has an id, so both are shown and only a missing request counts.
fn observed_line(requested: &str, stage: Stage, usage: Option<&Usage>) -> String {
    let Some(usage) = usage else {
        return "observed: not tracked: usage has not been collected for this run yet".into();
    };
    if usage.status == UsageStatus::NotTracked {
        let reason = usage.reason.as_deref().unwrap_or("no reason recorded");
        return format!("observed: not tracked: {}", shown(reason));
    }
    // Models in the order they first appear, each marked when subagents used it.
    let mut models: Vec<(&str, bool)> = Vec::new();
    for row in usage.rows.iter().filter(|row| row.stage == stage) {
        let subagents = !row.subagents.is_empty();
        match models.iter_mut().find(|(model, _)| *model == row.model) {
            Some((_, seen)) => *seen |= subagents,
            None => models.push((&row.model, subagents)),
        }
    }
    if models.is_empty() {
        return "observed: none recorded for this stage".into();
    }
    let list: Vec<String> = models
        .iter()
        .map(|(model, subagents)| {
            format!(
                "{}{}",
                shown(model),
                if *subagents { " (+subagents)" } else { "" }
            )
        })
        .collect();
    let verdict = if models
        .iter()
        .any(|(model, _)| model_matches(requested, model))
    {
        "matches the request".to_string()
    } else if usage.status == UsageStatus::Partial {
        format!(
            "{} not observed (usage is partial, so it may be missing)",
            shown(requested)
        )
    } else {
        format!(
            "MISMATCH: {} was requested and never observed",
            shown(requested)
        )
    };
    format!("observed: {}; {verdict}", list.join(", "))
}

pub(crate) fn render(run: &Run) -> String {
    let mut out = format!("Run {}: {}\n", run.id, shown(&run.title));
    out.push_str(&format!(
        "{:<14}{} ({}){}\n",
        "Stage",
        stage_name(run.stage),
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
    out.push_str(&field("Mode", run.mode.map(mode_name)));
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
    if let Some(ticket) = &run.current_ticket {
        out.push_str(&field("Current ticket", Some(ticket)));
    }
    if run.summaries.is_empty() {
        out.push_str("Summaries     none recorded\n");
    } else {
        out.push_str("Summaries\n");
        for (stage, text) in &run.summaries {
            out.push_str(&format!("  {}: {}\n", stage_name(*stage), shown(text)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_summary(summary: &str) -> Run {
        serde_json::from_value(serde_json::json!({
            "id": "1", "title": "t", "project": "/p", "stage": "build", "status": "running",
            "summaries": {"define": summary}
        }))
        .unwrap()
    }

    fn routed_run() -> Run {
        serde_json::from_value(serde_json::json!({
            "id": "1", "title": "t", "project": "/p", "stage": "build", "status": "running",
            "model_routing": {
                "agent": "claude",
                "roles": {
                    "strong": {"model": "opus", "effort": "high"},
                    "coding": {"model": "sonnet"}
                },
                "tasks": {"build": "coding", "check": "strong"}
            }
        }))
        .unwrap()
    }

    #[test]
    fn model_routing_is_shown_per_task_only_when_a_selection_exists() {
        let text = render(&routed_run());
        let lines: Vec<_> = text
            .lines()
            .skip_while(|line| !line.starts_with("Model routing"))
            .take(4)
            .collect();
        assert_eq!(
            lines,
            [
                "Model routing claude",
                "  build   coding  sonnet",
                "          observed: not tracked: usage has not been collected for this run yet",
                "  check   strong  opus, effort high",
            ],
            "{text}"
        );
        assert!(
            text.contains("  finish  not routed\n  Effort observed: unknown"),
            "{text}"
        );
        assert!(!render(&run_with_summary("s")).contains("Model routing"));
        // The machine-readable form is the whole run, so skills can read the selection from it.
        let json = serde_json::to_value(routed_run()).unwrap();
        assert_eq!(json["model_routing"]["tasks"]["check"], "strong");
        assert_eq!(json["model_routing"]["roles"]["strong"]["model"], "opus");
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
            "observed: claude-sonnet-5-5; matches the request"
        );
        // The coordinator ran on Sonnet, the review agents on Opus: the request is met.
        assert_eq!(
            observed(&text, "check"),
            "observed: claude-sonnet-5-5, claude-opus-5-5 (+subagents); matches the request"
        );
        assert!(!text.contains("MISMATCH"), "{text}");
        assert!(text.contains("Effort observed: unknown"), "{text}");
        assert!(
            text.contains("  check   strong  opus, effort high\n"),
            "{text}"
        );
    }

    #[test]
    fn a_request_that_was_never_observed_is_flagged_as_a_mismatch() {
        let text = render(&with_usage(
            "tracked",
            &[
                ("build", "claude-sonnet-5-5", false),
                ("check", "claude-sonnet-5-5", false),
            ],
        ));
        assert_eq!(
            observed(&text, "check"),
            "observed: claude-sonnet-5-5; MISMATCH: opus was requested and never observed"
        );
        assert!(
            observed(&text, "build").ends_with("matches the request"),
            "{text}"
        );
    }

    #[test]
    fn partial_usage_does_not_turn_a_missing_model_into_a_mismatch() {
        let text = render(&with_usage(
            "partial",
            &[("check", "claude-sonnet-5-5", false)],
        ));
        assert_eq!(
            observed(&text, "check"),
            "observed: claude-sonnet-5-5; opus not observed (usage is partial, so it may be missing)"
        );
        assert!(!text.contains("MISMATCH"), "{text}");
    }

    #[test]
    fn a_stage_without_usage_rows_and_a_run_without_coverage_say_so() {
        let text = render(&with_usage(
            "tracked",
            &[("build", "claude-sonnet-5-5", false)],
        ));
        assert_eq!(
            observed(&text, "check"),
            "observed: none recorded for this stage"
        );
        let untracked = render(&with_usage("not-tracked", &[]));
        assert_eq!(
            observed(&untracked, "build"),
            "observed: not tracked: no session logs were found"
        );
        assert!(!untracked.contains("MISMATCH"), "{untracked}");
    }

    #[test]
    fn a_task_that_is_not_routed_has_no_observed_line() {
        let text = render(&with_usage(
            "tracked",
            &[("finish", "claude-haiku-5-5", false)],
        ));
        let after: Vec<_> = text
            .lines()
            .skip_while(|l| !l.starts_with("  finish"))
            .take(2)
            .collect();
        assert_eq!(after[0], "  finish  not routed");
        assert!(after[1].starts_with("  Effort observed"), "{after:?}");
    }

    #[test]
    fn an_override_is_compared_with_the_role_it_chose() {
        let mut run = with_usage("tracked", &[("check", "claude-sonnet-5-5", false)]);
        run.model_routing
            .as_mut()
            .unwrap()
            .overrides
            .insert(Task::Check, crate::routing::Role::Coding);
        let text = render(&run);
        assert!(
            observed(&text, "check").ends_with("matches the request"),
            "{text}"
        );
    }

    #[test]
    fn an_overridden_task_shows_its_role_and_is_marked() {
        let mut run = routed_run();
        run.model_routing
            .as_mut()
            .unwrap()
            .overrides
            .insert(Task::Check, crate::routing::Role::Coding);
        let text = render(&run);
        assert!(
            text.contains("  check   coding  sonnet (override)\n"),
            "{text}"
        );
        assert!(text.contains("  build   coding  sonnet\n"), "{text}");
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
