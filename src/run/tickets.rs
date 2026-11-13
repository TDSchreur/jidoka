//! The ticket list of a `jidoka run update`: which tickets exist and how far each one is.

use super::{
    FieldUpdate, MAX_PATH_BYTES, MAX_TEXT_BYTES, Run, Ticket, TicketStatus, check_reference,
    check_text, repeated,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Tickets one update may list; a spec split into more pieces than this is too big to track on one board.
const MAX_TICKETS: usize = 100;

/// Help lines for the ticket flags, as a macro so `UPDATE_HELP` can stay a `const` built with `concat!`.
macro_rules! help {
    () => {
        concat!(
            "  --ticket \"<reference> <title>\" (repeat; the flags are the whole ordered list) | --clear-tickets\n",
            "  --ticket-status <reference>=<pending|in-progress|done> (repeat)\n"
        )
    };
}
pub(super) use help;

/// A ticket as listed by an update; its status is carried over from the stored list or starts pending.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewTicket {
    pub(crate) reference: String,
    pub(crate) title: String,
}

/// What an update does to the run's tickets.
#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct TicketsUpdate {
    /// The whole ordered ticket list; null removes it.
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) tickets: FieldUpdate<Vec<NewTicket>>,
    /// New statuses by ticket reference, applied after `tickets`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) ticket_status: BTreeMap<String, TicketStatus>,
}

impl TicketsUpdate {
    /// Reads `--ticket`, `--clear-tickets` or `--ticket-status` and the values after it.
    pub(super) fn parse_option<'a>(
        &mut self,
        flag: &str,
        next_value: &mut dyn FnMut() -> Result<&'a &'a str, String>,
    ) -> Result<(), String> {
        match flag {
            "--ticket" => {
                let value = *next_value()?;
                let (reference, title) = value
                    .split_once(char::is_whitespace)
                    .map(|(reference, title)| (reference, title.trim()))
                    .filter(|(_, title)| !title.is_empty())
                    .ok_or_else(|| {
                        format!("--ticket needs \"<reference> <title>\", got {value:?}")
                    })?;
                let ticket = NewTicket {
                    reference: reference.to_string(),
                    title: title.to_string(),
                };
                match &mut self.tickets {
                    FieldUpdate::Keep => self.tickets = FieldUpdate::Set(vec![ticket]),
                    FieldUpdate::Set(list) => list.push(ticket),
                    FieldUpdate::Clear => return Err(repeated(flag)),
                }
            }
            "--clear-tickets" => {
                if !self.tickets.is_keep() {
                    return Err(repeated(flag));
                }
                self.tickets = FieldUpdate::Clear;
            }
            _ => {
                let value = *next_value()?;
                let (reference, status) = value.rsplit_once('=').ok_or_else(|| {
                    format!("--ticket-status needs <reference>=<status>, got {value:?}")
                })?;
                let status: TicketStatus = serde_json::from_value(status.into()).map_err(|_| {
                    format!("Invalid ticket status {status:?}; use pending, in-progress or done")
                })?;
                if self
                    .ticket_status
                    .insert(reference.to_string(), status)
                    .is_some()
                {
                    return Err(format!("Ticket status for {reference} given twice"));
                }
            }
        }
        Ok(())
    }

    /// Checks the list limits, references and titles before anything is applied.
    pub(super) fn validate(&self) -> Result<(), String> {
        let FieldUpdate::Set(list) = &self.tickets else {
            return Ok(());
        };
        if list.len() > MAX_TICKETS {
            return Err(format!("At most {MAX_TICKETS} tickets can be listed"));
        }
        for (index, ticket) in list.iter().enumerate() {
            check_reference("Ticket reference", &ticket.reference, MAX_PATH_BYTES)?;
            if ticket.reference.chars().any(char::is_whitespace) {
                return Err(format!(
                    "Ticket reference {:?} must not contain spaces",
                    ticket.reference
                ));
            }
            check_text("Ticket title", &ticket.title, MAX_TEXT_BYTES)?;
            if list[..index]
                .iter()
                .any(|other| other.reference == ticket.reference)
            {
                return Err(format!("Ticket {} is listed twice", ticket.reference));
            }
        }
        Ok(())
    }

    /// Replaces or clears the list, then sets the statuses.
    pub(super) fn apply(self, run: &mut Run) -> Result<(), String> {
        match self.tickets {
            FieldUpdate::Keep => {}
            FieldUpdate::Clear => run.tickets.clear(),
            FieldUpdate::Set(list) => {
                let old = std::mem::take(&mut run.tickets);
                run.tickets = list
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
        for (reference, status) in self.ticket_status {
            let ticket = run
                .tickets
                .iter_mut()
                .find(|ticket| ticket.reference == reference)
                .ok_or_else(|| {
                    format!(
                        "No ticket {reference} is listed on this run; list it with --ticket first"
                    )
                })?;
            ticket.status = status;
        }
        if run
            .tickets
            .iter()
            .filter(|t| t.status == TicketStatus::InProgress)
            .count()
            > 1
        {
            return Err("Only one ticket can be in progress at a time".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{parse_update, tests::sample_run};

    fn parsed(options: &[&str]) -> Result<TicketsUpdate, String> {
        Ok(parse_update("running", options)?.tickets)
    }

    #[test]
    fn the_list_replaces_the_old_one_and_keeps_known_statuses() {
        let mut run = sample_run();
        parsed(&["--ticket", "a First", "--ticket-status", "a=done"])
            .unwrap()
            .apply(&mut run)
            .unwrap();
        parsed(&["--ticket", "b Second", "--ticket", "a First again"])
            .unwrap()
            .apply(&mut run)
            .unwrap();
        let listed: Vec<_> = run
            .tickets
            .iter()
            .map(|t| (t.reference.as_str(), t.status))
            .collect();
        assert_eq!(
            listed,
            [("b", TicketStatus::Pending), ("a", TicketStatus::Done)]
        );
    }

    #[test]
    fn conflicting_options_and_bad_lists_are_refused() {
        assert!(parsed(&["--clear-tickets", "--ticket", "a A"]).is_err());
        assert!(parsed(&["--ticket", "a A", "--clear-tickets"]).is_err());
        assert!(parsed(&["--ticket", "a A", "--ticket", "a B"]).is_err());
        assert!(parsed(&["--ticket-status", "a=done", "--ticket-status", "a=pending"]).is_err());
    }

    #[test]
    fn statuses_need_a_listed_ticket_and_only_one_can_run() {
        let mut run = sample_run();
        let unknown = parsed(&["--ticket-status", "zzz=done"]).unwrap();
        assert!(unknown.apply(&mut run).is_err());
        let two = parsed(&[
            "--ticket",
            "a A",
            "--ticket",
            "b B",
            "--ticket-status",
            "a=in-progress",
            "--ticket-status",
            "b=in-progress",
        ])
        .unwrap();
        assert!(two.apply(&mut run).is_err());
    }
}
