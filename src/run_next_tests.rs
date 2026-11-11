use super::*;
use crate::routing::{ModelChoice, ModelRouting, RoutingAgent};

fn run(mode: Mode, stage: Stage, routing: Option<ModelRouting>) -> Run {
    let mut run: Run = serde_json::from_value(serde_json::json!({
        "id": "1",
        "title": "Title",
        "project": "/work/repo",
        "stage": "define",
        "status": "running"
    }))
    .unwrap();
    run.model_routing = routing;
    run.mode = Some(mode);
    run.stage = stage;
    run.summaries.insert(Stage::Define, "the plan".into());
    run
}

fn routing() -> ModelRouting {
    ModelRouting {
        agent: RoutingAgent::Claude,
        stages: BTreeMap::from([(
            Stage::Build,
            ModelChoice {
                model: "sonnet".into(),
                effort: Some("medium".into()),
            },
        )]),
        overrides: BTreeMap::from([(
            Stage::Check,
            ModelChoice {
                model: "opus".into(),
                effort: None,
            },
        )]),
    }
}

fn names(gates: &[OpenGate]) -> Vec<Gate> {
    gates.iter().map(|g| g.name).collect()
}

#[test]
fn fast_build_lists_gates_and_has_no_checks_or_rework() {
    let next = build(&run(Mode::Fast, Stage::Build, None), &[]).unwrap();
    assert_eq!(names(&next.gates), [Gate::TicketsDone, Gate::Verified]);
    assert_eq!(next.moves.len(), 1);
    assert_eq!(next.moves[0].to, Some(Stage::Finish));
    assert!(next.checks.is_none());
    assert_eq!(next.summaries[&Stage::Define], "the plan");
}

#[test]
fn balanced_check_offers_advance_and_rework_with_applicable_checks() {
    let next = build(
        &run(Mode::Balanced, Stage::Check, None),
        &["src/a.rs".into()],
    )
    .unwrap();
    let kinds: Vec<_> = next.moves.iter().map(|m| (m.kind, m.to)).collect();
    assert_eq!(
        kinds,
        [
            ("stage", Some(Stage::Finish)),
            ("rework", Some(Stage::Build))
        ]
    );
    let checks = next.checks.unwrap();
    assert_eq!(checks.applicable[0].name, "code-review");
}

#[test]
fn full_build_requires_per_ticket_verification_and_finish_ends_in_done() {
    let build_next = build(&run(Mode::Full, Stage::Build, None), &[]).unwrap();
    assert!(names(&build_next.gates).contains(&Gate::TicketsVerified));
    let finish = build(&run(Mode::Full, Stage::Finish, None), &[]).unwrap();
    assert_eq!(finish.moves[0].kind, "done");
    assert_eq!(finish.moves[0].to, None);
}

#[test]
fn model_comes_from_the_override_then_the_policy_and_is_none_without_routing() {
    let build_next = build(&run(Mode::Balanced, Stage::Build, Some(routing())), &[]).unwrap();
    assert_eq!(build_next.model.unwrap().model, "sonnet");
    let check = build(&run(Mode::Balanced, Stage::Check, Some(routing())), &[]).unwrap();
    assert_eq!(check.model.unwrap().model, "opus");
    let define = build(&run(Mode::Balanced, Stage::Define, Some(routing())), &[]).unwrap();
    assert!(define.model.is_none());
    for mode in [Mode::Fast, Mode::Balanced, Mode::Full] {
        assert!(
            build(&run(mode, Stage::Build, None), &[])
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
    let next = build(&blocked, &[]).unwrap();
    assert!(next.moves.is_empty());
    assert!(next.note.unwrap().contains("blocked"));
}

#[test]
fn a_run_without_mode_is_an_error() {
    let mut unknown = run(Mode::Fast, Stage::Build, None);
    unknown.mode = None;
    assert!(build(&unknown, &[]).is_err());
}

#[test]
fn json_and_text_show_the_model_and_gates() {
    let next = build(&run(Mode::Balanced, Stage::Build, Some(routing())), &[]).unwrap();
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
