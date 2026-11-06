//! The readable form of a run, for `factory run inspect`: everything a new session needs to continue it.

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
