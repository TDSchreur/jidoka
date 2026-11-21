//! Intentional status changes and the policy for returning from Check to Build.

use super::{
    FindingState, MAX_EVENTS, MAX_NOTE_BYTES, MAX_REWORK, PauseKind, Run, RunEvent, RunEventKind,
    RunStatus, UpdateRun, check_text,
};
use crate::stages::{self, Destination};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LifecycleSource {
    Agent,
    Pause,
    PullRequest,
    Workspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FindingClassification {
    Wording,
    Behavior,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ReworkRecord {
    pub(crate) number: u32,
    pub(crate) at: u64,
    pub(crate) finding: String,
    pub(crate) findings: Vec<String>,
    pub(crate) classification: FindingClassification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) decision: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum LifecycleRequest {
    Block {
        reason: String,
    },
    Resume {
        source: String,
    },
    Rework {
        finding: String,
        classification: FindingClassification,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<String>,
    },
    WorkspaceBlocked {
        reason: String,
    },
    WorkspaceReady {
        update: Box<UpdateRun>,
    },
}

/// ADR-0006 requires a decision for behavior changes in the third Rework.
const WORDING_REWORK: u32 = 3;

impl Run {
    pub(crate) fn block(
        &mut self,
        reason: &str,
        source: LifecycleSource,
        at: u64,
    ) -> Result<(), String> {
        if self.status != RunStatus::Running {
            return Err("Only a running run can be blocked".into());
        }
        self.set_block(reason, source, at)
    }

    pub(crate) fn resume(
        &mut self,
        reason: &str,
        source: LifecycleSource,
        at: u64,
    ) -> Result<(), String> {
        if self.status != RunStatus::Blocked {
            return Err("Only a blocked run can be resumed".into());
        }
        check_text("Resume source", reason, MAX_NOTE_BYTES)?;
        self.status = RunStatus::Running;
        self.blocking_reason = None;
        self.block_source = None;
        self.lifecycle_event(RunEventKind::Resumed, reason, source, at);
        Ok(())
    }

    pub(crate) fn automatically_blocked(&self) -> bool {
        self.status == RunStatus::Blocked
            && match self.block_source {
                Some(source) => source == LifecycleSource::Pause,
                None => self
                    .blocking_reason
                    .as_deref()
                    .is_some_and(|reason| PauseKind::from_reason(reason).is_some()),
            }
    }

    pub(super) fn refresh_block(
        &mut self,
        reason: &str,
        source: LifecycleSource,
        at: u64,
    ) -> Result<(), String> {
        // Only the automation that owns a block may refresh its reason.
        if (source == LifecycleSource::Pause && self.automatically_blocked())
            || (self.status == RunStatus::Blocked && self.block_source == Some(source))
        {
            self.set_block(reason, source, at)?;
        }
        Ok(())
    }

    fn set_block(&mut self, reason: &str, source: LifecycleSource, at: u64) -> Result<(), String> {
        check_text("Blocking reason", reason, MAX_NOTE_BYTES)?;
        self.status = RunStatus::Blocked;
        self.blocking_reason = Some(reason.to_string());
        self.block_source = Some(source);
        self.lifecycle_event(RunEventKind::Blocked, reason, source, at);
        Ok(())
    }

    fn lifecycle_event(
        &mut self,
        kind: RunEventKind,
        reason: &str,
        source: LifecycleSource,
        at: u64,
    ) {
        if self.events.len() < MAX_EVENTS {
            self.events.push(RunEvent {
                at,
                kind,
                stage: self.stage,
                reason: Some(reason.to_string()),
                source: Some(source),
            });
        }
    }

    pub(super) fn start_rework(
        &mut self,
        finding: String,
        classification: FindingClassification,
        decision: Option<String>,
        at: u64,
    ) -> Result<(), String> {
        let mode = self
            .mode
            .ok_or("Record the run's mode before starting a Rework")?;
        let transition = stages::rework(mode, self.stage, self.status)?;
        let number = next_rework(self.rework)?;
        if let Some(decision) = &decision {
            check_text("Rework decision", decision, MAX_NOTE_BYTES)?;
        }
        if (number > WORDING_REWORK
            || (number == WORDING_REWORK && classification == FindingClassification::Behavior))
            && decision.is_none()
        {
            return Err(format!(
                "Rework {number} requires the user's decision; record it with --decision"
            ));
        }
        let findings: Vec<_> = self
            .findings
            .iter()
            .filter(|finding| finding.state == FindingState::Open)
            .map(|finding| finding.id.clone())
            .collect();
        if !findings.contains(&finding) {
            return Err(format!(
                "Rework requires a recorded open finding; {finding} is not open"
            ));
        }

        let Destination::Stage(stage) = transition.to else {
            return Err("A Rework must lead to Build".into());
        };
        for open in self
            .findings
            .iter_mut()
            .filter(|finding| finding.state == FindingState::Open)
        {
            open.rework = Some(number);
        }
        self.stage = stage;
        self.rework = Some(number);
        self.rework_history.push(ReworkRecord {
            number,
            at,
            finding,
            findings,
            classification,
            decision,
        });
        Ok(())
    }
}

pub(crate) fn next_rework(current: Option<u32>) -> Result<u32, String> {
    let count = current.unwrap_or_default();
    if count >= MAX_REWORK {
        return Err(format!(
            "The run has reached the limit of {MAX_REWORK} Reworks"
        ));
    }
    Ok(count + 1)
}

pub(crate) fn parse_lifecycle(action: &str, options: &[&str]) -> Result<LifecycleRequest, String> {
    match (action, options) {
        ("block", ["--reason", reason]) => {
            check_text("Blocking reason", reason, MAX_NOTE_BYTES)?;
            Ok(LifecycleRequest::Block {
                reason: (*reason).into(),
            })
        }
        ("resume", ["--source", source]) => {
            check_text("Resume source", source, MAX_NOTE_BYTES)?;
            Ok(LifecycleRequest::Resume {
                source: (*source).into(),
            })
        }
        ("rework", _) => parse_rework(options),
        ("block", _) => Err("Use jidoka run block <id> --reason <text>".into()),
        ("resume", _) => Err("Use jidoka run resume <id> --source <text>".into()),
        _ => Err(format!("Unknown lifecycle action: {action}")),
    }
}

fn parse_rework(options: &[&str]) -> Result<LifecycleRequest, String> {
    let usage = || {
        "Use jidoka run rework <id> --finding <id> --classification <wording|behavior> [--decision <text>]".to_string()
    };
    let mut finding = None;
    let mut classification = None;
    let mut decision = None;
    let (pairs, extra) = options.as_chunks::<2>();
    if !extra.is_empty() {
        return Err(usage());
    }
    for pair in pairs {
        match pair {
            ["--finding", value] if finding.is_none() => finding = Some((*value).into()),
            ["--classification", value] if classification.is_none() => {
                classification = Some(match *value {
                    "wording" => FindingClassification::Wording,
                    "behavior" => FindingClassification::Behavior,
                    _ => return Err(usage()),
                });
            }
            ["--decision", value] if decision.is_none() => {
                check_text("Rework decision", value, MAX_NOTE_BYTES)?;
                decision = Some((*value).into());
            }
            _ => return Err(usage()),
        }
    }
    Ok(LifecycleRequest::Rework {
        finding: finding.ok_or_else(usage)?,
        classification: classification.ok_or_else(usage)?,
        decision,
    })
}
