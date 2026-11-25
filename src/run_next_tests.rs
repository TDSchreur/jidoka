use super::*;
use crate::routing::{ModelChoice, ModelRouting};
use std::path::Path;

fn run(mode: Mode, stage: Stage, routing: Option<ModelRouting>) -> Run {
    let mut run: Run = serde_json::from_value(serde_json::json!({
        "id": "1",
        "title": "Title",
        "gates": [], "project": "/work/repo",
        "stage": "define",
        "status": "running"
    }))
    .unwrap();
    run.model_routing = routing;
    run.tickets.push(Ticket {
        reference: "ticket".into(),
        title: "A ticket".into(),
        status: TicketStatus::Pending,
        work_attempt: 0,
    });
    run.mode = Some(mode);
    run.stage = stage;
    run.worktree = Some("/work/repo".into());
    run.base_commit = Some("abc".into());
    run.summaries.insert(Stage::Define, "the plan".into());
    run
}

fn routing() -> ModelRouting {
    ModelRouting {
        agent: Agent::Claude,
        stages: BTreeMap::from([(
            Stage::Build,
            ModelChoice {
                model: "sonnet".into(),
                effort: Some("medium".into()),
            },
        )]),
        complex: BTreeMap::new(),
        prior_choices: BTreeMap::new(),
        overrides: BTreeMap::from([(
            Stage::Check,
            ModelChoice {
                model: "opus".into(),
                effort: None,
            },
        )]),
    }
}

struct LocalGit;

impl Git for LocalGit {
    fn git(&self, _: &Path, args: &[&str]) -> Result<String, String> {
        match args {
            ["rev-parse", "HEAD"] => Ok("def".into()),
            ["diff", ..] => Ok("src/a.rs\0".into()),
            _ => panic!("unexpected Git operation: {args:?}"),
        }
    }
}

fn build(run: &Run) -> Result<Next, String> {
    super::build(run, &LocalGit)
}

fn names(gates: &[OpenGate]) -> Vec<Gate> {
    gates.iter().map(|g| g.name).collect()
}

#[test]
fn fast_build_lists_gates_and_has_no_checks_or_rework() {
    let next = build(&run(Mode::Fast, Stage::Build, None)).unwrap();
    assert_eq!(names(&next.gates), [Gate::TicketsDone, Gate::Verified]);
    assert_eq!(next.moves.len(), 1);
    assert_eq!(next.moves[0].to, Some(Stage::Finish));
    assert!(next.checks.is_none());
    assert_eq!(next.summaries[&Stage::Define], "the plan");
}

#[test]
fn balanced_check_offers_advance_with_applicable_checks() {
    let next = build(&run(Mode::Balanced, Stage::Check, None)).unwrap();
    let kinds: Vec<_> = next.moves.iter().map(|m| (m.kind, m.to)).collect();
    assert_eq!(kinds, [("stage", Some(Stage::Finish))]);
    let checks = next.checks.unwrap();
    assert_eq!(checks.applicable[0].name, "code-review");
}

#[test]
fn full_build_requires_tickets_and_finish_ends_in_done() {
    let build_next = build(&run(Mode::Full, Stage::Build, None)).unwrap();
    assert_eq!(
        names(&build_next.gates),
        [Gate::TicketsDone, Gate::TicketsVerified]
    );
    let finish = build(&run(Mode::Full, Stage::Finish, None)).unwrap();
    assert_eq!(finish.moves[0].kind, "done");
    assert_eq!(finish.moves[0].to, None);
}

#[test]
fn model_comes_from_the_override_then_the_policy_and_is_none_without_routing() {
    let build_next = build(&run(Mode::Balanced, Stage::Build, Some(routing()))).unwrap();
    assert_eq!(build_next.model.unwrap().model, "sonnet");
    let check = build(&run(Mode::Balanced, Stage::Check, Some(routing()))).unwrap();
    assert_eq!(check.model.unwrap().model, "opus");
    let define = build(&run(Mode::Balanced, Stage::Define, Some(routing()))).unwrap();
    assert!(define.model.is_none());
    for mode in [Mode::Fast, Mode::Balanced, Mode::Full] {
        assert!(
            build(&run(mode, Stage::Build, None))
                .unwrap()
                .model
                .is_none()
        );
    }
}

#[test]
fn blocked_run_has_a_note_and_no_moves() {
    let mut blocked = run(Mode::Fast, Stage::Build, None);
    blocked.status = RunStatus::Blocked;
    let next = build(&blocked).unwrap();
    assert!(next.moves.is_empty());
    assert!(next.note.unwrap().contains("blocked"));
}

#[test]
fn a_run_without_mode_is_an_error() {
    let mut unknown = run(Mode::Fast, Stage::Build, None);
    unknown.mode = None;
    assert!(build(&unknown).is_err());
}

#[test]
fn json_and_text_show_the_model_and_gates() {
    let next = build(&run(Mode::Balanced, Stage::Build, Some(routing()))).unwrap();
    let json = serde_json::to_value(&next).unwrap();
    assert_eq!(json["stage"], "build");
    assert_eq!(json["model"]["effort"], "medium");
    assert_eq!(json["gates"][0]["name"], "tickets-done");
    let text = render(&next);
    assert!(text.contains("Model sonnet, effort medium"));
    assert!(text.contains("open  tickets-done"));
}

#[test]
fn options_accept_only_json() {
    assert_eq!(parse_options(&[]), Ok(false));
    assert_eq!(parse_options(&["--json"]), Ok(true));
    assert!(parse_options(&["--x"]).is_err());
}

#[test]
fn next_shows_recorded_results_and_only_open_review_gates() {
    let mut run = run(Mode::Balanced, Stage::Check, None);
    run.gates.push(crate::run::GateResult {
        gate: Gate::ChecksRecorded,
        stage: Stage::Check,
        rework: None,
        passed: true,
        evidence: Some("reviewed".into()),
        commit: None,
        ticket: None,
        work_attempt: None,
        check: Some("code-review".into()),
        skipped_checks: Vec::new(),
    });
    let next = build(&run).unwrap();
    assert_eq!(
        names(&next.gates),
        [
            Gate::ChecksRecorded,
            Gate::NoActionableFindings,
            Gate::Verified
        ]
    );
    assert!(render(&next).contains("pass  checks-recorded (check code-review): reviewed"));
    assert_eq!(
        serde_json::to_value(&next).unwrap()["gate_results"][0]["passed"],
        true
    );
    run.rework = Some(1);
    assert_eq!(
        names(&build(&run).unwrap().gates),
        [
            Gate::ChecksRecorded,
            Gate::NoActionableFindings,
            Gate::Verified
        ]
    );
}

#[test]
fn next_supplies_only_current_stage_and_routing_when_selected() {
    for mode in [Mode::Fast, Mode::Balanced, Mode::Full] {
        for &stage in stages::stages(mode) {
            let next = build(&run(mode, stage, None)).unwrap();
            assert!(next.instructions.contains(&format!(
                "{}: {}",
                match mode {
                    Mode::Fast => "Fast",
                    Mode::Balanced => "Balanced",
                    Mode::Full => "Full",
                },
                match stage {
                    Stage::Define => "Define",
                    Stage::Build => "Build",
                    Stage::Check => "Check",
                    Stage::Finish => "Finish",
                }
            )));
            assert!(!next.instructions.contains("# Model routing"));
            assert!(!next.instructions.contains("No instructions for this stage"));
        }
    }
    let routed = build(&run(Mode::Balanced, Stage::Build, Some(routing()))).unwrap();
    assert!(routed.instructions.contains("# Model routing"));
}

#[test]
fn build_instructions_are_focused_and_full_explains_each_ticket_gate() {
    let output = build(&run(Mode::Full, Stage::Build, Some(routing()))).unwrap();
    assert!(output.instructions.contains("tickets-verified pass"));
    assert!(
        output
            .instructions
            .contains("--ticket-status <reference>=in-progress")
    );
    assert!(!output.instructions.contains("## Committing for Check"));
    assert!(!output.instructions.contains("## Reworks"));
    assert!(!output.instructions.contains("On Copilot CLI"));
    assert!(
        output.instructions.len() < 8000,
        "stage guidance unexpectedly large"
    );
    let check = build(&run(Mode::Balanced, Stage::Check, None)).unwrap();
    assert!(check.instructions.contains("## Committing for Check"));
    for mode in [Mode::Balanced, Mode::Full] {
        let check = build(&run(mode, Stage::Check, None)).unwrap();
        assert!(check.instructions.contains("## Check reviews"));
        assert!(check.instructions.contains("no-actionable-findings pass"));
    }
}

#[test]
fn codex_approvals_are_shown_only_to_codex_or_an_unknown_agent() {
    let shown = |routing: Option<ModelRouting>| {
        build(&run(Mode::Balanced, Stage::Finish, routing))
            .unwrap()
            .instructions
            .contains("## Codex command approvals")
    };
    let with = |agent| ModelRouting { agent, ..routing() };
    assert!(!shown(Some(with(Agent::Claude))));
    assert!(!shown(Some(with(Agent::Copilot))));
    assert!(shown(Some(with(Agent::Codex))));
    assert!(shown(None));
}

#[test]
fn finish_includes_the_pull_request_procedure_and_a_done_run_has_no_instructions() {
    let finish = build(&run(Mode::Balanced, Stage::Finish, None)).unwrap();
    assert!(finish.instructions.contains("## Pull request and CI"));
    let mut done = run(Mode::Balanced, Stage::Finish, None);
    done.status = RunStatus::Done;
    let next = build(&done).unwrap();
    assert!(next.instructions.is_empty());
    assert!(!render(&next).contains("Instructions"));
}

#[test]
fn applicable_check_supplies_instructions_gate_and_routed_model() {
    let next = build(&run(Mode::Balanced, Stage::Check, Some(routing()))).unwrap();
    assert!(next.instructions.contains("## Code review"));
    assert!(next.instructions.contains("Use `jidoka-code-review`"));
    let json = serde_json::to_value(&next).unwrap();
    assert_eq!(json["checks"]["applicable"][0]["gate"], "checks-recorded");
    assert_eq!(json["checks"]["applicable"][0]["model"]["model"], "opus");
    assert!(render(&next).contains("gate: checks-recorded"));
    let build_stage = build(&run(Mode::Balanced, Stage::Build, Some(routing()))).unwrap();
    assert!(!build_stage.instructions.contains("## Code review"));
}

struct FailingDiff;

impl Git for FailingDiff {
    fn git(&self, _dir: &Path, _args: &[&str]) -> Result<String, String> {
        Err("git diff failed".into())
    }
}

#[test]
fn check_selection_explains_unavailable_evidence_only_when_needed() {
    let mut run = run(Mode::Balanced, Stage::Build, None);
    run.worktree = None;
    let next = super::build(&run, &FailingDiff).unwrap();
    assert!(
        next.checks.is_none(),
        "Build guidance included Check selection: {:?}",
        next.checks
    );
    assert!(
        next.checks_unavailable.is_none(),
        "Build guidance reported Check selection unavailable: {:?}",
        next.checks_unavailable
    );

    run.stage = Stage::Check;
    let reason = |run: &Run| {
        let next = super::build(run, &FailingDiff).unwrap();
        assert!(
            next.checks.is_none(),
            "Check guidance showed a selection without a usable diff: {:?}",
            next.checks
        );
        let reason = next.checks_unavailable.unwrap();
        let gate = next
            .gates
            .iter()
            .find(|g| g.name == Gate::ChecksRecorded)
            .unwrap();
        assert_eq!(gate.reason.as_deref(), Some(reason.as_str()));
        reason
    };
    assert_eq!(reason(&run), "Record the worktree to select checks");
    run.worktree = Some("/work/repo".into());
    run.base_commit = None;
    assert_eq!(reason(&run), "Record the base commit to select checks");
    run.base_commit = Some("abc".into());
    assert_eq!(
        reason(&run),
        "Could not inspect the changed paths in /work/repo: git diff failed"
    );
}

#[test]
fn forced_check_exit_is_distinct_from_a_recorded_review_pass() {
    let mut run = run(Mode::Balanced, Stage::Check, None);
    run.gates.push(crate::run::GateResult {
        gate: Gate::ChecksRecorded,
        stage: Stage::Check,
        rework: None,
        passed: true,
        evidence: Some("only one check".into()),
        commit: Some("abc".into()),
        ticket: None,
        work_attempt: None,
        check: Some("code-review".into()),
        skipped_checks: vec![],
    });
    run.leave(
        crate::run::Exit::Override {
            reason: "review incomplete".into(),
        },
        1,
    )
    .unwrap();
    let next = build(&run).unwrap();
    let json = serde_json::to_value(&next).unwrap();
    assert_eq!(json["stage_exits"][0]["outcome"], "override");
    let review = &json["stage_exits"][0]["gates"][0];
    assert_eq!(review["name"], "checks-recorded");
    assert_eq!(review["status"], "bypassed");
    assert!(review.get("results").is_none());
    assert!(render(&next).contains("Override (review incomplete)"));
}

#[test]
fn finish_guidance_waits_for_live_ci_even_with_a_stored_pr_state() {
    let mut run = run(Mode::Fast, Stage::Finish, None);
    run.pr_status = Some(crate::pr_state::PrStatus {
        url: "https://example.test/pr/1".into(),
        state: crate::pr_state::PrState::Merged,
        reason: None,
        checked_at: 0,
    });
    let next = build(&run).unwrap();
    let json = serde_json::to_value(&next).unwrap();
    assert_eq!(json["gates"][1]["name"], "ci-passed");
    assert_eq!(json["gates"][1]["status"], "awaiting-live-check");
    run.leave(
        crate::run::Exit::Merged {
            url: "https://example.test/pr/1".into(),
        },
        1,
    )
    .unwrap();
    let next = build(&run).unwrap();
    let json = serde_json::to_value(&next).unwrap();
    assert_eq!(json["stage_exits"][0]["outcome"], "merged");
    assert_eq!(json["stage_exits"][0]["gates"][1]["status"], "bypassed");
}

#[test]
fn rework_and_finish_use_effective_build_for_each_host() {
    for agent in [Agent::Claude, Agent::Codex, Agent::Copilot] {
        for (effort, expected) in [
            (None, "high"),
            (Some("medium"), "high"),
            (Some("high"), "high"),
            (Some("xhigh"), "xhigh"),
            (Some("max"), "max"),
            (Some("ultra"), "ultra"),
            (Some("future"), "future"),
        ] {
            let mut routes = routing();
            routes.agent = agent;
            routes.stages.get_mut(&Stage::Build).unwrap().effort = effort.map(str::to_string);
            let mut building = run(Mode::Balanced, Stage::Build, Some(routes.clone()));
            building.rework = Some(1);
            let next = build(&building).unwrap();
            assert_eq!(next.model.unwrap().effort.as_deref(), Some(expected));
            let mut finishing = run(Mode::Balanced, Stage::Finish, Some(routes));
            finishing.rework = Some(1);
            let next = build(&finishing).unwrap();
            assert!(next.model.is_none());
            assert_eq!(next.ci_model.unwrap().effort.as_deref(), Some(expected));
            assert!(next.instructions.contains("CI diagnosis"));
        }
    }
}

#[test]
fn complex_policy_option_is_shown_and_override_wins() {
    let mut routes = routing();
    routes.complex.insert(
        Stage::Build,
        ModelChoice {
            model: "opus".into(),
            effort: Some("high".into()),
        },
    );
    let next = build(&run(Mode::Balanced, Stage::Build, Some(routes.clone()))).unwrap();
    assert_eq!(next.complex_model.unwrap().model, "opus");
    routes.overrides.insert(
        Stage::Build,
        ModelChoice {
            model: "custom".into(),
            effort: Some("xhigh".into()),
        },
    );
    let mut current = run(Mode::Balanced, Stage::Build, Some(routes));
    current.rework = Some(1);
    assert_eq!(build(&current).unwrap().model.unwrap().model, "custom");
}

fn group_view(spec: &str) -> crate::groups::GroupView {
    serde_json::from_value(serde_json::json!({
        "id": "g1",
        "title": "Bicep v2.4.0",
        "spec": spec,
        "summary": "1 of 2 attached PRs merged, 1 milestone pending",
        "members": [
            {"run": "1", "missing": false, "dependencies": [
                {"id": "d2", "target": {"pr": "7"}, "description": "pull request of run 7", "cleared": true, "status": "merged"},
                {"id": "d3", "target": {"milestone": "m1"}, "description": "m1: v2.4.0 published", "cleared": false, "status": "pending"}
            ]},
            {"run": "7", "missing": false}
        ]
    }))
    .unwrap()
}

#[test]
fn finish_shows_the_group_spec_and_the_outstanding_prerequisites() {
    let guidance = GroupGuidance::from_view(&group_view("https://example.com/spec"), "1");
    let next = build(&run(Mode::Full, Stage::Finish, None))
        .unwrap()
        .with_group(guidance);
    let text = render(&next);
    assert!(text.contains("Change group g1: Bicep v2.4.0"), "{text}");
    assert!(text.contains("Spec  https://example.com/spec"), "{text}");
    assert!(text.contains("1 outstanding of 2"), "{text}");
    assert!(text.contains("d2  clear"), "{text}");
    assert!(
        text.contains("d3  pending  m1: v2.4.0 published - pending"),
        "{text}"
    );
    let json = serde_json::to_value(&next).unwrap();
    assert_eq!(json["group"]["outstanding"], 1);
}

#[test]
fn a_local_spec_path_is_kept_out_of_the_guidance() {
    let guidance = GroupGuidance::from_view(&group_view("/home/me/plan.md"), "1").unwrap();
    assert_eq!(guidance.spec, None);
    let next = build(&run(Mode::Full, Stage::Finish, None))
        .unwrap()
        .with_group(Some(guidance));
    let text = render(&next);
    assert!(text.contains("a local file"), "{text}");
    assert!(!text.contains("/home/me/plan.md"), "{text}");
    assert!(!serde_json::to_string(&next).unwrap().contains("plan.md"));
}

#[test]
fn an_ungrouped_run_or_a_non_member_gets_no_group_section() {
    assert!(GroupGuidance::from_view(&group_view("https://example.com"), "99").is_none());
    let next = build(&run(Mode::Full, Stage::Finish, None)).unwrap();
    assert!(!render(&next).contains("\nChange group g"));
    assert!(serde_json::to_value(&next).unwrap().get("group").is_none());
}

#[test]
fn knowledge_guidance_appears_only_when_configured() {
    let plain = build(&run(Mode::Full, Stage::Define, None)).unwrap();
    let before = render(&plain);
    assert!(!before.contains("Knowledge repo"));
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("knowledge"));

    let setting = crate::knowledge::Knowledge {
        path: Some("/work/knowledge".into()),
        ..Default::default()
    };
    let guidance = crate::knowledge::Guidance::for_setting(&setting);
    let next = build(&run(Mode::Full, Stage::Define, None))
        .unwrap()
        .with_knowledge(guidance);
    let text = render(&next);
    // Everything the unconfigured output says is still there.
    assert!(before.lines().all(|line| text.contains(line)));
    assert!(text.contains("Knowledge repo checkout: /work/knowledge"));
    assert!(text.contains("**Define.**") && text.contains("**Finish.**"));
    assert!(text.contains("Never record secrets"));
}
