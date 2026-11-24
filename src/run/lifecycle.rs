//! Intentional status changes: blocking and resuming a run, and the lifecycle requests.

use super::{
    MAX_EVENTS, MAX_NOTE_BYTES, PauseKind, Run, RunEvent, RunEventKind, RunStatus, UpdateRun,
    check_text,
};
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

impl FindingClassification {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Wording => "wording",
            Self::Behavior => "behavior",
        }
    }
}

/// The Coordinator's classification of the proposed fix for one Finding.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassifiedFinding {
    pub(crate) id: String,
    pub(crate) classification: FindingClassification,
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
        findings: Vec<ClassifiedFinding>,
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

    /// Whether a blocked run may require a person's attention.
    pub(crate) fn needs_you(&self) -> bool {
        self.status == RunStatus::Blocked
            && !(self.automatically_blocked()
                && self.blocking_reason.as_deref() == Some(PauseKind::Compaction.reason()))
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
        "Use jidoka run rework <id> --finding <id>=<wording|behavior> ... [--decision <text>]; include each open Finding once".to_string()
    };
    let mut findings = Vec::new();
    let mut decision = None;
    let (pairs, extra) = options.as_chunks::<2>();
    if !extra.is_empty() {
        return Err(usage());
    }
    for pair in pairs {
        match pair {
            ["--finding", value] => {
                let (id, classification) = value.split_once('=').ok_or_else(usage)?;
                let classification = match classification {
                    "wording" => FindingClassification::Wording,
                    "behavior" => FindingClassification::Behavior,
                    _ => return Err(usage()),
                };
                if id.is_empty() {
                    return Err(usage());
                }
                findings.push(ClassifiedFinding {
                    id: id.to_owned(),
                    classification,
                });
            }
            ["--decision", value] if decision.is_none() => {
                check_text("Rework decision", value, MAX_NOTE_BYTES)?;
                decision = Some((*value).into());
            }
            _ => return Err(usage()),
        }
    }
    if findings.is_empty() {
        return Err(usage());
    }
    Ok(LifecycleRequest::Rework { findings, decision })
}
