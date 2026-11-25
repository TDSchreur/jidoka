//! `jidoka run next`: what a run does now, in the order the agent needs it.

use crate::{
    agent::Agent,
    checks,
    git::Git,
    inspect::shown,
    routing::{ModelChoice, ModelRouting},
    run::{
        Evaluation, ExitOutcome, GateResult, GateStatus, Mode, ReworkFindings, Run, RunStatus,
        Stage, StageExit, Ticket, TicketStatus,
    },
    run_gate::{self, AssessmentPurpose, CheckSelection},
    stages::{self, Destination, Gate},
};
use serde::Serialize;
use std::collections::BTreeMap;

/// A Gate that is still open, with what satisfies it.
#[derive(Clone, Serialize, Debug, PartialEq, Eq)]
pub(crate) struct OpenGate {
    name: Gate,
    description: &'static str,
    status: GateStatus,
    reason: Option<String>,
}

/// A legal move out of the current Stage.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct Move {
    /// `stage`, `done` or `rework`.
    kind: &'static str,
    to: Option<Stage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rework: Option<u32>,
    gates: Vec<OpenGate>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct CheckEntry {
    name: &'static str,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    gate: Option<Gate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<ModelChoice>,
}

/// The Checks of the Check stage: those that apply to the diff and those skipped.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct Checks {
    applicable: Vec<CheckEntry>,
    skipped: Vec<CheckEntry>,
}

/// What the run's change group asks of its Finish: the shared spec and the conditions the run waits on.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct GroupGuidance {
    group: String,
    title: String,
    /// The shared spec's web link; absent for a local file, whose path stays out of pull requests.
    spec: Option<String>,
    prerequisites: Vec<Prerequisite>,
    outstanding: usize,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct Prerequisite {
    id: String,
    description: String,
    status: String,
    cleared: bool,
}

impl GroupGuidance {
    /// The guidance for member `run` of the group in `view`; none when it is not a member.
    pub(crate) fn from_view(view: &crate::groups::GroupView, run: &str) -> Option<Self> {
        let member = view.members.iter().find(|member| member.run == run)?;
        let lower = view.spec.to_ascii_lowercase();
        let web = lower.starts_with("http://") || lower.starts_with("https://");
        let prerequisites: Vec<Prerequisite> = member
            .dependencies
            .iter()
            .map(|dependency| Prerequisite {
                id: dependency.id.clone(),
                description: dependency.description.clone(),
                status: dependency.status.clone(),
                cleared: dependency.cleared,
            })
            .collect();
        Some(Self {
            group: view.id.clone(),
            title: view.title.clone(),
            spec: web.then(|| view.spec.clone()),
            outstanding: prerequisites.iter().filter(|p| !p.cleared).count(),
            prerequisites,
        })
    }
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
    gate_results: Vec<GateResult>,
    stage_exits: Vec<StageExit>,
    checks_unavailable: Option<String>,
    moves: Vec<Move>,
    /// Only in the Check stage.
    checks: Option<Checks>,
    summaries: BTreeMap<Stage, String>,
    tickets: Vec<Ticket>,
    unticketed_attempt: Option<u64>,
    /// The routed model for this Stage; none when routing is off or leaves the Stage out.
    model: Option<ModelChoice>,
    /// Optional complex choice and Finish's CI repair route.
    complex_model: Option<ModelChoice>,
    ci_model: Option<ModelChoice>,
    model_reason: Option<&'static str>,
    /// Why there is no move, when the run is blocked or done.
    note: Option<String>,
    /// Only in Finish, and only for a run in a change group.
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<GroupGuidance>,
    /// Only in Define and Finish, and only when a knowledge repo is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge: Option<crate::knowledge::Guidance>,
}

impl Next {
    /// Adds the knowledge repo guidance.
    pub(crate) fn with_knowledge(mut self, knowledge: Option<crate::knowledge::Guidance>) -> Self {
        self.knowledge = knowledge;
        self
    }

    /// Adds the run's change group guidance.
    pub(crate) fn with_group(mut self, group: Option<GroupGuidance>) -> Self {
        self.group = group;
        self
    }
}

fn describe(gate: Gate) -> &'static str {
    match gate {
        Gate::TicketsDone => "every listed ticket is done, or the run declares unticketed work",
        Gate::Verified => "the full verification passed on the head commit",
        Gate::TicketsVerified => {
            "verification passed for each ticket attempt or for the unticketed Build"
        }
        Gate::ChecksRecorded => "every applicable check has a recorded result",
        Gate::NoActionableFindings => {
            "the checks left no actionable findings; otherwise start a Rework"
        }
        Gate::FinishSummary => "a Finish summary is recorded",
        Gate::CiPassed => "CI passed on the pull request's head commit, or the project has no CI",
    }
}

fn open(evaluations: Vec<Evaluation>) -> Vec<OpenGate> {
    evaluations
        .into_iter()
        .filter(|gate| gate.status != GateStatus::Passed)
        .map(|gate| OpenGate {
            name: gate.name,
            description: describe(gate.name),
            status: gate.status,
            reason: gate.reason,
        })
        .collect()
}

#[derive(Serialize)]
pub(crate) struct StageGates<'a> {
    stage: Stage,
    rework: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit: Option<&'a ExitOutcome>,
    gates: Vec<GateDisplay>,
}

#[derive(Serialize)]
struct GateDisplay {
    #[serde(flatten)]
    evaluation: Evaluation,
    description: &'static str,
}

fn gate_display(evaluations: Vec<Evaluation>) -> Vec<GateDisplay> {
    evaluations
        .into_iter()
        .map(|evaluation| GateDisplay {
            description: describe(evaluation.name),
            evaluation,
        })
        .collect()
}

/// Show saved exits and live local results for the current pass.
pub(crate) fn stage_gates(run: &Run, mut current: Vec<Evaluation>) -> Vec<StageGates<'_>> {
    let Some(mode) = run.mode else {
        return Vec::new();
    };
    let history = run.stage_exits.iter().map(|exit| StageGates {
        stage: exit.stage,
        rework: exit.rework,
        exit: Some(&exit.outcome),
        gates: gate_display(exit.gates.clone()),
    });
    let remaining = stages::stages(mode).iter().filter_map(|&stage| {
        let transition = stages::next(mode, stage, RunStatus::Running).ok()?;
        let active = stage == run.stage && run.status != RunStatus::Done;
        if !active && run.stage_exits.iter().any(|exit| exit.stage == stage) {
            return None;
        }
        let gates = if active {
            std::mem::take(&mut current)
        } else {
            transition
                .gates
                .iter()
                .map(|&gate| Evaluation::with_status(gate, GateStatus::Unknown))
                .collect()
        };
        Some(StageGates {
            stage,
            rework: active.then_some(run.rework).flatten(),
            exit: None,
            gates: gate_display(gates),
        })
    });
    history.chain(remaining).collect()
}

include!(concat!(env!("OUT_DIR"), "/stage_chunks.rs"));

/// `agent` is the run's coding agent when known; `routing` is the agent only when this stage is routed.
fn instructions(mode: Mode, stage: Stage, agent: Option<Agent>, routing: Option<Agent>) -> String {
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
        Stage::Finish => &["stage-reporting", "finish", "pull-request-and-ci"],
    };
    let routed = routing.map(|agent| match agent {
        Agent::Claude => "routing-claude",
        Agent::Codex => "routing-codex",
        Agent::Copilot => "routing-copilot",
    });
    let stage_routing = routing.map(|_| match stage {
        Stage::Define => "routing-define",
        Stage::Build => "routing-build",
        Stage::Check => "routing-check",
        Stage::Finish => "routing-finish",
    });
    // Only Codex needs its approval rules; show them when the agent is unknown too.
    let codex_approvals = matches!(agent, None | Some(Agent::Codex)).then_some("codex-approvals");
    let reporting = shared.iter().copied().take(1);
    let rest = shared.iter().copied().skip(1);
    for name in reporting
        .chain(codex_approvals)
        .chain(rest)
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

pub(crate) fn checks_for(
    mode: Mode,
    selection: &checks::Selection,
    routing: Option<&ModelRouting>,
) -> Checks {
    Checks {
        applicable: selection
            .applicable
            .iter()
            .map(|a| CheckEntry {
                name: a.check.name,
                gate: Some(a.check.gate.gate()),
                model: a
                    .check
                    .model_route
                    .and_then(|stage| routing.and_then(|r| r.choice_for(stage)))
                    .cloned(),
                reason: match &a.reason {
                    checks::Reason::Floor => format!("{} always runs it", mode.name()),
                    checks::Reason::Triggered { path } => format!("{} changed", shown(path)),
                },
            })
            .collect(),
        skipped: selection
            .skipped
            .iter()
            .map(|s| CheckEntry {
                name: s.check.name,
                gate: None,
                model: None,
                reason: s.reason.clone(),
            })
            .collect(),
    }
}

fn rework_move(run: &Run, mode: Mode) -> Option<Move> {
    let rework = stages::rework(mode, run.stage, run.status).ok()?;
    run.open_findings().next()?;
    let Destination::Stage(stage) = rework.to else {
        return None;
    };
    Some(Move {
        kind: "rework",
        to: Some(stage),
        rework: Some(crate::run::next_rework(run.rework).ok()?),
        gates: Vec::new(),
    })
}

fn recorded_gate_results(run: &Run) -> Vec<GateResult> {
    run.gates
        .iter()
        .filter(|r| {
            r.stage == run.stage
                && (r.rework == run.rework
                    || (r.gate == Gate::TicketsVerified
                        && run.tickets.iter().any(|ticket| {
                            r.ticket.as_deref() == Some(&ticket.reference)
                                && r.work_attempt == Some(ticket.work_attempt)
                        })))
        })
        .cloned()
        .collect()
}

/// Builds stage guidance and the moves available to the run.
///
/// # Errors
///
/// Returns a message when the run has no recorded mode.
pub(crate) fn build(run: &Run, git: &dyn Git) -> Result<Next, String> {
    let mode = run.mode.ok_or_else(|| {
        "the run has no recorded mode; record it with `jidoka run update <id> --mode <mode>`"
            .to_string()
    })?;
    let mut moves = Vec::new();
    let mut gates = Vec::new();
    let mut note = None;
    let assessment = run_gate::assess_local(run, git, AssessmentPurpose::Guidance);
    match assessment.transition {
        Ok(transition) => {
            gates = open(assessment.evaluations);
            let (kind, to) = match transition.to {
                Destination::Stage(stage) => ("stage", Some(stage)),
                Destination::Done => ("done", None),
            };
            moves.push(Move {
                kind,
                to,
                rework: None,
                gates: gates.clone(),
            });
            moves.extend(rework_move(run, mode));
        }
        Err(reason) => note = Some(reason),
    }
    let (selection, checks_unavailable) = match &assessment.checks {
        CheckSelection::NotApplicable => (None, None),
        CheckSelection::Unavailable(reason) => (None, Some(reason.clone())),
        CheckSelection::Selected { selection, .. } => (Some(selection), None),
    };
    Ok(Next {
        run: run.id.clone(),
        title: run.title.clone(),
        mode,
        stage: run.stage,
        status: run.status,
        rework: run.rework,
        instructions: stage_instructions(run, mode, selection),
        gates,
        gate_results: recorded_gate_results(run),
        stage_exits: run.stage_exits.clone(),
        checks_unavailable,
        moves,
        checks: selection.map(|selection| checks_for(mode, selection, run.model_routing.as_ref())),
        summaries: run.summaries.clone(),
        tickets: run.tickets.clone(),
        unticketed_attempt: run.unticketed_attempt,
        model: run
            .model_routing
            .as_ref()
            .and_then(|r| r.effective(run.stage, run.rework)),
        complex_model: run
            .model_routing
            .as_ref()
            .and_then(|r| r.complex.get(&run.stage))
            .cloned(),
        ci_model: (run.stage == Stage::Finish)
            .then(|| {
                run.model_routing
                    .as_ref()
                    .and_then(|r| r.effective(Stage::Build, run.rework))
            })
            .flatten(),
        model_reason: (run.stage == Stage::Build
            && run.rework.is_some()
            && run
                .model_routing
                .as_ref()
                .is_some_and(|r| r.choice_for(Stage::Build).is_some()))
        .then_some("Build rework; known lower effort raised to high"),
        note,
        group: None,
        knowledge: None,
    })
}

fn stage_instructions(run: &Run, mode: Mode, selection: Option<&checks::Selection>) -> String {
    if run.status == RunStatus::Done {
        return String::new();
    }
    let mut stage_instructions = instructions(
        mode,
        run.stage,
        run.model_routing.as_ref().map(|routing| routing.agent),
        run.model_routing
            .as_ref()
            .filter(|routing| {
                routing.choice_for(run.stage).is_some()
                    || (run.stage == Stage::Finish && routing.choice_for(Stage::Build).is_some())
            })
            .map(|routing| routing.agent),
    );
    if let Some(selection) = selection {
        for applicable in &selection.applicable {
            stage_instructions.push_str("\n\n");
            stage_instructions.push_str(applicable.check.instructions);
        }
    }
    stage_instructions
}

fn render_tickets(next: &Next, out: &mut String) {
    if next.unticketed_attempt.is_some() {
        out.push_str("\nTickets: unticketed work\n");
    } else if !next.tickets.is_empty() {
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
}

#[expect(
    clippy::too_many_lines,
    reason = "formats every section of the next-step instructions as one long string builder"
)]
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
    if let Some(reason) = next.model_reason {
        out.push_str(&format!(
            "  {reason} (stronger or unknown effort preserved)\n"
        ));
    }
    if let Some(choice) = &next.complex_model {
        out.push_str(&format!(
            "Complex option {}, effort {}\n",
            shown(&choice.model),
            choice.effort.as_deref().map_or("unspecified".into(), shown)
        ));
    }
    if next.stage == Stage::Finish {
        let route = next
            .ci_model
            .as_ref()
            .map_or("not routed".into(), |choice| {
                format!(
                    "{}{}",
                    shown(&choice.model),
                    choice
                        .effort
                        .as_deref()
                        .map_or(String::new(), |e| format!(", effort {}", shown(e)))
                )
            });
        out.push_str(&format!("CI repair model {route}\n"));
    }
    if !next.instructions.is_empty() {
        out.push_str(&format!("\nInstructions\n  {}\n", next.instructions));
    }
    out.extend(next.group.as_ref().map(render_group));
    out.extend(next.knowledge.as_ref().map(|knowledge| {
        format!(
            "\nKnowledge repo checkout: {}\n\n{}\n",
            shown(&knowledge.path),
            knowledge.instructions.trim_end()
        )
    }));
    if let Some(reason) = &next.checks_unavailable {
        out.push_str(&format!(
            "\nCheck selection unavailable: {}\n",
            shown(reason)
        ));
    }
    if let Some(checks) = &next.checks {
        out.push_str("\nChecks\n");
        for entry in &checks.applicable {
            out.push_str(&format!(
                "  run   {} ({})\n",
                entry.name,
                shown(&entry.reason)
            ));
            if let Some(gate) = entry.gate {
                out.push_str(&format!("        gate: {}\n", gate.name()));
            }
            if let Some(model) = &entry.model {
                out.push_str(&format!(
                    "        model: {}{}\n",
                    shown(&model.model),
                    model
                        .effort
                        .as_deref()
                        .map_or(String::new(), |e| format!(", effort {}", shown(e)))
                ));
            }
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
            render_result(&mut out, result, "  ");
        }
    }
    render_exits(&mut out, &next.stage_exits);
    if let Some(note) = &next.note {
        out.push_str(&format!("\nNo move: {note}\n"));
    }
    for mv in &next.moves {
        let target = mv.to.map_or("done", Stage::name);
        out.push_str(&format!("\nMove to {target} ({})\n", mv.kind));
        if let Some(number) = mv.rework {
            out.push_str(&format!(
                "  Rework {number}: jidoka run rework {} --finding <id>=<wording|behavior> ... [--decision <text>] (one for each open Finding)\n",
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
                "  {}  {}: {}\n",
                gate_status_label(gate.status),
                gate.name.name(),
                gate.reason
                    .as_deref()
                    .map_or_else(|| gate.description.to_owned(), shown)
            ));
        }
    }
    render_tickets(next, &mut out);
    if !next.summaries.is_empty() {
        out.push_str("\nSummaries\n");
        for (stage, text) in &next.summaries {
            out.push_str(&format!("  {}: {}\n", stage.name(), shown(text)));
        }
    }
    out
}

fn render_result(out: &mut String, result: &GateResult, indent: &str) {
    out.push_str(&format!(
        "{indent}{}  {}{}{}{}{}{}\n",
        if result.passed { "pass" } else { "fail" },
        result.gate.name(),
        result
            .check
            .as_ref()
            .map_or(String::new(), |s| format!(" (check {})", shown(s))),
        result
            .ticket
            .as_ref()
            .map_or(String::new(), |s| format!(" (ticket {})", shown(s))),
        result
            .work_attempt
            .map_or(String::new(), |attempt| format!(" (attempt {attempt})")),
        result
            .commit
            .as_ref()
            .map_or(String::new(), |commit| format!(" at {}", shown(commit))),
        result
            .evidence
            .as_ref()
            .map_or(String::new(), |s| format!(": {}", shown(s)))
    ));
}

fn gate_status_label(status: GateStatus) -> &'static str {
    match status {
        GateStatus::Open => "open",
        GateStatus::Failed => "failed",
        GateStatus::Passed => "passed",
        GateStatus::AwaitingLiveCheck => "awaiting live check",
        GateStatus::Bypassed => "bypassed",
        GateStatus::Unknown => "unknown",
    }
}

fn render_exits(out: &mut String, exits: &[StageExit]) {
    if exits.is_empty() {
        return;
    }
    out.push_str("\nRecorded stage exits\n");
    for exit in exits {
        let pass = exit
            .rework
            .map_or_else(String::new, |n| format!(" (Rework {n})"));
        let outcome = match &exit.outcome {
            ExitOutcome::Accepted => "Accepted".to_string(),
            ExitOutcome::Override { reason } => format!("Override ({})", shown(reason)),
            ExitOutcome::Merged { url } => format!("PR merge observed ({})", shown(url)),
            ExitOutcome::Rework { number, included } => {
                let ids = match included {
                    ReworkFindings::Classified { findings } => findings
                        .iter()
                        .map(|f| format!("{}={}", f.id, f.classification.label()))
                        .collect::<Vec<_>>()
                        .join(", "),
                    ReworkFindings::Historical { finding, .. } => finding.clone(),
                };
                format!("Started Rework {number} ({})", shown(&ids))
            }
        };
        out.push_str(&format!("  {}{pass}: {outcome}\n", exit.stage.name()));
        for gate in &exit.gates {
            let status = gate_status_label(gate.status);
            out.push_str(&format!("    {}: {status}\n", gate.name.name()));
            if let Some(detail) = &gate.detail {
                out.push_str(&format!("      {}\n", shown(detail)));
            }
            for result in &gate.results {
                render_result(out, result, "      ");
            }
        }
    }
}

/// The change group section of `run next`.
fn render_group(group: &GroupGuidance) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\nChange group {}: {}\n",
        group.group,
        shown(&group.title)
    ));
    match &group.spec {
            Some(spec) => out.push_str(&format!("  Spec  {}\n", shown(spec))),
            None => out.push_str(
                "  Spec  a local file: describe the prerequisites in the pull request and leave its path out\n",
            ),
        }
    if group.prerequisites.is_empty() {
        out.push_str("  Prerequisites  none\n");
    } else {
        out.push_str(&format!(
            "  Prerequisites  {} outstanding of {}\n",
            group.outstanding,
            group.prerequisites.len()
        ));
        for item in &group.prerequisites {
            out.push_str(&format!(
                "    {}  {}  {} - {}\n",
                item.id,
                if item.cleared { "clear  " } else { "pending" },
                shown(&item.description),
                shown(&item.status)
            ));
        }
    }
    out
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
