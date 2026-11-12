//! `jidoka run next`: what a run does now, in the order the agent needs it.

use crate::{
    checks,
    inspect::shown,
    pr_sync::Git,
    routing::{ModelChoice, RoutingAgent},
    run::{Mode, Run, RunStatus, Stage, Ticket, TicketStatus},
    stages::{self, Destination, Gate},
};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

/// A Gate that is still open, with what satisfies it.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct OpenGate {
    name: Gate,
    description: &'static str,
}

/// A legal move out of the current Stage.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct Move {
    /// `stage`, `done` or `rework`.
    kind: &'static str,
    to: Option<Stage>,
    gates: Vec<OpenGate>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct CheckEntry {
    name: &'static str,
    reason: String,
}

/// The Checks of the Check stage: those that apply to the diff and those skipped.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct Checks {
    applicable: Vec<CheckEntry>,
    skipped: Vec<CheckEntry>,
}

#[derive(Serialize, Debug)]
pub(crate) struct Next {
    run: String,
    title: String,
    mode: Mode,
    stage: Stage,
    status: RunStatus,
    rework: Option<u32>,
    instructions: String,
    /// The gates still open on the move to the next Stage.
    gates: Vec<OpenGate>,
    gate_results: Vec<crate::run_gate::GateResult>,
    moves: Vec<Move>,
    /// Only in the Check stage.
    checks: Option<Checks>,
    summaries: BTreeMap<Stage, String>,
    tickets: Vec<Ticket>,
    /// The routed model for this Stage; none when routing is off or leaves the Stage out.
    model: Option<ModelChoice>,
    /// Why there is no move, when the run is blocked or done.
    note: Option<String>,
}

fn describe(gate: Gate) -> &'static str {
    match gate {
        Gate::TicketsDone => {
            "every ticket is done (jidoka run update <id> running --ticket-status <ref>=done)"
        }
        Gate::Verified => "the full verification passed on the head commit",
        Gate::TicketsVerified => "verification passed after each ticket",
        Gate::ChecksRecorded => "every applicable check has a recorded result",
        Gate::NoActionableFindings => {
            "the checks left no actionable findings; otherwise start a Rework"
        }
        Gate::CiPassed => "CI passed on the pull request's head commit, or the project has no CI",
    }
}

fn open(run: &Run, gates: &[Gate]) -> Vec<OpenGate> {
    gates
        .iter()
        .filter(|&&gate| match gate {
            Gate::TicketsDone => !run.tickets.iter().all(|t| t.status == TicketStatus::Done),
            Gate::ChecksRecorded | Gate::NoActionableFindings => {
                !crate::run_gate::latest(run, gate).is_some_and(|r| r.passed)
            }
            Gate::Verified => !crate::run_gate::latest(run, gate).is_some_and(|r| {
                r.passed
                    && crate::run_gate::head(run, &crate::run_check::ReadOnlyGit)
                        .ok()
                        .as_ref()
                        == r.commit.as_ref()
            }),
            Gate::TicketsVerified => !run.tickets.iter().all(|t| {
                run.gates
                    .iter()
                    .rev()
                    .find(|r| {
                        r.gate == gate
                            && r.stage == run.stage
                            && r.ticket.as_deref() == Some(&t.reference)
                    })
                    .is_some_and(|r| r.passed)
            }),
            Gate::CiPassed => true,
        })
        .map(|&name| OpenGate {
            name,
            description: describe(name),
        })
        .collect()
}

/// A snapshot of each stage's exit gates. Unrecorded automatic checks are unknown outside
/// the current stage; advancing does not persist their outcomes.
#[derive(Serialize)]
pub(crate) struct StageGates {
    stage: Stage,
    gates: Vec<GateStatus>,
}

#[derive(Serialize)]
struct GateStatus {
    name: Gate,
    description: &'static str,
    status: &'static str,
    evidence: Option<String>,
    rework: Option<u32>,
}

pub(crate) fn stage_gates(run: &Run) -> Vec<StageGates> {
    let Some(mode) = run.mode else {
        return Vec::new();
    };
    stages::stages(mode)
        .iter()
        .map(|&stage| {
            let transition =
                stages::next(mode, stage, RunStatus::Running).expect("stage belongs to mode");
            let current = stage == run.stage && run.status != RunStatus::Done;
            let open_gates = if current {
                open(run, transition.gates)
            } else {
                Vec::new()
            };
            let gates = transition
                .gates
                .iter()
                .map(|&gate| {
                    let result = run.gates.iter().rev().find(|r| {
                        r.stage == stage && r.gate == gate && (!current || r.rework == run.rework)
                    });
                    let status = if gate == Gate::CiPassed {
                        "unknown"
                    } else if current {
                        if open_gates.iter().any(|g| g.name == gate) {
                            if result.is_some_and(|r| !r.passed) {
                                "failed"
                            } else {
                                "open"
                            }
                        } else {
                            "passed"
                        }
                    } else if let Some(result) = result {
                        if result.passed {
                            "recorded-pass"
                        } else {
                            "failed"
                        }
                    } else {
                        "unknown"
                    };
                    GateStatus {
                        name: gate,
                        description: describe(gate),
                        status,
                        evidence: result.and_then(|r| r.evidence.clone()),
                        rework: result.and_then(|r| r.rework),
                    }
                })
                .collect();
            StageGates { stage, gates }
        })
        .collect()
}

include!(concat!(env!("OUT_DIR"), "/stage_chunks.rs"));

fn instructions(mode: Mode, stage: Stage, routing: Option<RoutingAgent>) -> String {
    let mode_dir = match mode {
        Mode::Fast => "fast",
        Mode::Balanced => "balanced",
        Mode::Full => "full",
    };
    let path = format!("{mode_dir}/{}.md", stage.name());
    let stage_text = STAGE_CHUNKS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| std::str::from_utf8(bytes).expect("stage chunk is UTF-8"))
        .unwrap_or("No instructions for this stage in this mode.");
    let mut output = stage_text.to_owned();
    let shared: &[&str] = match stage {
        Stage::Define => &["stage-reporting", "define-rules", "tracker-setup"],
        Stage::Build => &["stage-reporting", "verification", "skill-change"],
        Stage::Check => &[
            "stage-reporting",
            "verification",
            "committing-for-check",
            "check-review",
        ],
        Stage::Finish => &["stage-reporting", "finish"],
    };
    let routed = routing.map(|agent| match agent {
        RoutingAgent::Claude => "routing-claude",
        RoutingAgent::Codex => "routing-codex",
        RoutingAgent::Copilot => "routing-copilot",
    });
    let stage_routing = routing.map(|_| match stage {
        Stage::Define => "routing-define",
        Stage::Build => "routing-build",
        Stage::Check => "routing-check",
        Stage::Finish => "routing-finish",
    });
    for name in shared
        .iter()
        .copied()
        .chain(routing.map(|_| "stage-routing"))
        .chain(routed)
        .chain(stage_routing)
    {
        let path = format!("jidoka-procedures/chunks/{name}.md");
        let chunk = crate::install::bundled_skill_text(&path).expect("shared procedure is bundled");
        output.push_str("\n\n");
        output.push_str(chunk);
    }
    output
}

pub(crate) fn checks_for(mode: Mode, changed_paths: &[String]) -> Checks {
    let selection = checks::select(mode, changed_paths);
    Checks {
        applicable: selection
            .applicable
            .into_iter()
            .map(|a| CheckEntry {
                name: a.check.name,
                reason: match a.reason {
                    checks::Reason::Floor => format!("{} always runs it", mode.name()),
                    checks::Reason::Triggered { path } => format!("{} changed", shown(&path)),
                },
            })
            .collect(),
        skipped: selection
            .skipped
            .into_iter()
            .map(|s| CheckEntry {
                name: s.check.name,
                reason: s.reason,
            })
            .collect(),
    }
}

/// The paths changed between the run's base commit and its head, when the run records both.
pub(crate) fn changed_paths(run: &Run, git: &dyn Git) -> Vec<String> {
    let (Some(worktree), Some(base)) = (&run.worktree, &run.base_commit) else {
        return Vec::new();
    };
    let range = format!("{base}..HEAD");
    git.git(Path::new(worktree), &["diff", "--name-only", &range])
        .map(|out| out.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// # Errors
///
/// Returns a message when the run has no recorded mode.
pub(crate) fn build(run: &Run, changed_paths: &[String]) -> Result<Next, String> {
    let mode = run.mode.ok_or_else(|| {
        "the run has no recorded mode; record it with `jidoka run update <id> running --mode <mode>`"
            .to_string()
    })?;
    let mut moves = Vec::new();
    let mut gates = Vec::new();
    let mut note = None;
    match stages::next(mode, run.stage, run.status) {
        Ok(transition) => {
            gates = open(run, transition.gates);
            let (kind, to) = match transition.to {
                Destination::Stage(stage) => ("stage", Some(stage)),
                Destination::Done => ("done", None),
            };
            moves.push(Move {
                kind,
                to,
                gates: open(run, transition.gates),
            });
            if let Ok(rework) = stages::rework(mode, run.stage, run.status)
                && let Destination::Stage(stage) = rework.to
            {
                moves.push(Move {
                    kind: "rework",
                    to: Some(stage),
                    gates: open(run, rework.gates),
                });
            }
        }
        Err(reason) => note = Some(reason),
    }
    Ok(Next {
        run: run.id.clone(),
        title: run.title.clone(),
        mode,
        stage: run.stage,
        status: run.status,
        rework: run.rework,
        instructions: instructions(
            mode,
            run.stage,
            run.model_routing
                .as_ref()
                .filter(|routing| routing.choice_for(run.stage).is_some())
                .map(|routing| routing.agent),
        ),
        gates,
        gate_results: run
            .gates
            .iter()
            .filter(|r| r.stage == run.stage && r.rework == run.rework)
            .cloned()
            .collect(),
        moves,
        checks: (run.stage == Stage::Check && mode != Mode::Fast)
            .then(|| checks_for(mode, changed_paths)),
        summaries: run.summaries.clone(),
        tickets: run.tickets.clone(),
        model: run
            .model_routing
            .as_ref()
            .and_then(|routing| routing.choice_for(run.stage))
            .cloned(),
        note,
    })
}

pub(crate) fn render(next: &Next) -> String {
    let mut out = format!("Run {}: {}\n", next.run, shown(&next.title));
    out.push_str(&format!(
        "Stage {} ({}), {}{}\n",
        next.stage.name(),
        match next.status {
            RunStatus::Running => "running",
            RunStatus::Blocked => "blocked",
            RunStatus::Done => "done",
        },
        next.mode.name(),
        next.rework
            .map_or(String::new(), |n| format!(", rework {n}"))
    ));
    match &next.model {
        Some(choice) => out.push_str(&format!(
            "Model {}{}\n",
            shown(&choice.model),
            choice
                .effort
                .as_deref()
                .map_or(String::new(), |e| format!(", effort {}", shown(e)))
        )),
        None => out.push_str("Model not routed\n"),
    }
    out.push_str(&format!("\nInstructions\n  {}\n", next.instructions));
    if let Some(checks) = &next.checks {
        out.push_str("\nChecks\n");
        for entry in &checks.applicable {
            out.push_str(&format!(
                "  run   {} ({})\n",
                entry.name,
                shown(&entry.reason)
            ));
        }
        for entry in &checks.skipped {
            out.push_str(&format!(
                "  skip  {} ({})\n",
                entry.name,
                shown(&entry.reason)
            ));
        }
    }
    if !next.gate_results.is_empty() {
        out.push_str("\nRecorded gate results\n");
        for result in &next.gate_results {
            out.push_str(&format!(
                "  {}  {}{}{}\n",
                if result.passed { "pass" } else { "fail" },
                shown_name(result.gate),
                result
                    .ticket
                    .as_ref()
                    .map_or(String::new(), |s| format!(" (ticket {})", shown(s))),
                result
                    .evidence
                    .as_ref()
                    .map_or(String::new(), |s| format!(": {}", shown(s)))
            ));
        }
    }
    if let Some(note) = &next.note {
        out.push_str(&format!("\nNo move: {note}\n"));
    }
    for mv in &next.moves {
        let target = mv.to.map_or("done", Stage::name);
        out.push_str(&format!("\nMove to {target} ({})\n", mv.kind));
        if mv.kind == "rework" {
            out.push_str(&format!(
                "  with: jidoka run update {} running --rework <n> --activity \"<findings>\"\n",
                next.run
            ));
        } else {
            out.push_str(&format!("  with: jidoka run advance {}\n", next.run));
        }
        if mv.gates.is_empty() {
            out.push_str("  no gates\n");
        }
        for gate in &mv.gates {
            out.push_str(&format!(
                "  open  {}: {}\n",
                shown_name(gate.name),
                gate.description
            ));
        }
    }
    if !next.tickets.is_empty() {
        out.push_str("\nTickets\n");
        for ticket in &next.tickets {
            let mark = match ticket.status {
                TicketStatus::Pending => "pending",
                TicketStatus::InProgress => "in-progress",
                TicketStatus::Done => "done",
            };
            out.push_str(&format!(
                "  [{mark}] {} {}\n",
                shown(&ticket.reference),
                shown(&ticket.title)
            ));
        }
    }
    if !next.summaries.is_empty() {
        out.push_str("\nSummaries\n");
        for (stage, text) in &next.summaries {
            out.push_str(&format!("  {}: {}\n", stage.name(), shown(text)));
        }
    }
    out
}

fn shown_name(gate: Gate) -> String {
    serde_json::to_value(gate)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub(crate) fn parse_options(options: &[&str]) -> Result<bool, String> {
    match options {
        [] => Ok(false),
        ["--json"] => Ok(true),
        _ => Err("Invalid arguments.".to_string()),
    }
}

#[cfg(test)]
#[path = "run_next_tests.rs"]
mod tests;
