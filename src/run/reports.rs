//! The reports of a `jidoka run update`: what skills and activities a run or one of its tickets used.

use super::{
    MAX_PATH_BYTES, MAX_REPORTS_PER_RUN, MAX_TEXT_BYTES, ReportKind, Run, RunReport, TicketReport,
    check_reference, check_text, has_control,
};
use serde::{Deserialize, Serialize};

/// Ticket reports or run reports one update may submit; matches the ticket limit, one report per ticket.
const MAX_REPORTS_PER_UPDATE: usize = 100;

/// Help lines for the report flags, as a macro so `UPDATE_HELP` can stay a `const` built with `concat!`.
macro_rules! help {
    () => {
        "  --ticket-report <reference> <skill|activity> <text> (repeat)\n  --run-report <skill|activity> <text> (repeat)\n"
    };
}
pub(super) use help;

/// A report on the run as a whole, as submitted by an update.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewRunReport {
    pub(crate) kind: ReportKind,
    pub(crate) text: String,
}

/// A report on one listed ticket, as submitted by an update.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewTicketReport {
    pub(crate) ticket: String,
    pub(crate) kind: ReportKind,
    pub(crate) text: String,
}

/// What an update adds to the run's reports.
#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct ReportsUpdate {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) ticket_reports: Vec<NewTicketReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) run_reports: Vec<NewRunReport>,
}

impl ReportsUpdate {
    /// Reads `--ticket-report` or `--run-report` and the values after it.
    pub(super) fn parse_option<'a>(
        &mut self,
        flag: &str,
        next_value: &mut dyn FnMut() -> Result<&'a &'a str, String>,
    ) -> Result<(), String> {
        if flag == "--run-report" {
            let kind = *next_value()?;
            let text = *next_value()?;
            let kind = serde_json::from_value(kind.into())
                .map_err(|_| "Run report kind must be skill or activity")?;
            self.run_reports.push(NewRunReport {
                kind,
                text: text.to_string(),
            });
            return Ok(());
        }
        let reference = *next_value()?;
        let kind = *next_value()?;
        let text = *next_value()?;
        let kind = serde_json::from_value(kind.into())
            .map_err(|_| "Ticket report kind must be skill or activity")?;
        self.ticket_reports.push(NewTicketReport {
            ticket: reference.to_string(),
            kind,
            text: text.to_string(),
        });
        Ok(())
    }

    /// Checks the limits, references and texts before anything is applied.
    pub(super) fn validate(&self) -> Result<(), String> {
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
        Ok(())
    }

    /// Adds the reports in the run's current stage; ticket reports need a listed ticket.
    pub(super) fn apply(self, run: &mut Run, at: u64) -> Result<(), String> {
        if run.ticket_reports.len() + self.ticket_reports.len() > MAX_REPORTS_PER_RUN {
            return Err(format!(
                "A run can hold at most {MAX_REPORTS_PER_RUN} ticket reports"
            ));
        }
        for report in self.ticket_reports {
            if !run
                .tickets
                .iter()
                .any(|ticket| ticket.reference == report.ticket)
            {
                return Err(format!("No ticket {} is listed on this run", report.ticket));
            }
            run.ticket_reports.push(TicketReport {
                ticket: report.ticket,
                kind: report.kind,
                text: report.text,
                stage: run.stage,
                at,
            });
        }
        if run.run_reports.len() + self.run_reports.len() > MAX_REPORTS_PER_RUN {
            return Err(format!(
                "A run can hold at most {MAX_REPORTS_PER_RUN} run reports"
            ));
        }
        for report in self.run_reports {
            run.run_reports.push(RunReport {
                kind: report.kind,
                text: report.text,
                stage: run.stage,
                at,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{Stage, parse_update, tests::sample_run};

    fn parsed(options: &[&str]) -> Result<ReportsUpdate, String> {
        Ok(parse_update("running", options)?.reports)
    }

    #[test]
    fn reports_are_stamped_with_the_stage_and_time() {
        let mut run = sample_run();
        run.stage = Stage::Build;
        run.tickets.push(crate::run::Ticket {
            reference: "a".into(),
            title: "A".into(),
            status: crate::run::TicketStatus::Pending,
        });
        parsed(&[
            "--ticket-report",
            "a",
            "skill",
            "rust",
            "--run-report",
            "activity",
            "Built",
        ])
        .unwrap()
        .apply(&mut run, 9)
        .unwrap();
        assert_eq!(
            (run.ticket_reports[0].stage, run.ticket_reports[0].at),
            (Stage::Build, 9)
        );
        assert_eq!(run.run_reports[0].text, "Built");
    }

    #[test]
    fn a_ticket_report_needs_a_listed_ticket_and_kinds_are_checked() {
        let mut run = sample_run();
        let update = parsed(&["--ticket-report", "zzz", "skill", "rust"]).unwrap();
        assert!(update.apply(&mut run, 1).is_err());
        assert!(parsed(&["--run-report", "other", "text"]).is_err());
        assert!(parsed(&["--run-report", "skill", "bad\u{7}text"]).is_err());
    }
}
