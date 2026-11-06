//! The readable form of a run, for `factory run inspect`: everything a new session needs to continue it.

use crate::routing::{ModelRouting, Task};
use crate::run::{Mode, Run, RunStatus, Stage, TicketStatus};

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
fn routing_section(routing: &ModelRouting) -> String {
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
    }
    out
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
        out.push_str(&routing_section(routing));
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
                "  check   strong  opus, effort high",
                "  finish  not routed",
            ],
            "{text}"
        );
        assert!(!render(&run_with_summary("s")).contains("Model routing"));
        // The machine-readable form is the whole run, so skills can read the selection from it.
        let json = serde_json::to_value(routed_run()).unwrap();
        assert_eq!(json["model_routing"]["tasks"]["check"], "strong");
        assert_eq!(json["model_routing"]["roles"]["strong"]["model"], "opus");
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
