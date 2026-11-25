use super::*;
use crate::test_support::{TestDir, advance_to, force_advance, new_run};

#[test]
fn generic_updates_cannot_complete_finish_with_or_without_a_summary() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    advance_to(&store, &run.id, Stage::Finish);
    assert!(parse_update(&["done", "--summary", "finish=Completed"]).is_err());
    assert!(serde_json::from_str::<UpdateRun>(r#"{"status":"done"}"#).is_err());
    let after = store.get(&run.id).unwrap().unwrap();
    assert!(after.finished_at.is_none() && after.summaries.is_empty());
    assert!(after.completion.is_none());
}

#[test]
fn agent_homes_are_recorded_per_agent_and_the_latest_wins() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let home = |name: &str| dir.0.path().join(name).to_str().unwrap().to_owned();
    let created = store
        .create(NewRun {
            title: "task".into(),
            project: "/work".into(),
            repository: None,
            session: Some("11111111-aaaa".into()),
            agent_homes: BTreeMap::from([("claude".into(), home(".claude"))]),
            model_routing: None,
        })
        .unwrap();
    assert_eq!(created.agent_homes["claude"], home(".claude"));

    let codex = format!("codex={}", home(".codex"));
    let work = format!("claude={}", home(".claude-work"));
    let updated = store
        .update(
            &created.id,
            parse_update(&["--agent-home", &codex, "--agent-home", &work]).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.agent_homes,
        BTreeMap::from([
            ("claude".into(), home(".claude-work")),
            ("codex".into(), home(".codex")),
        ])
    );
    // A record without agent homes leaves the field out.
    let plain = new_run(&store, "plain");
    assert!(
        serde_json::to_value(&plain)
            .unwrap()
            .get("agent_homes")
            .is_none()
    );
}

#[test]
fn an_agent_home_must_be_a_known_agent_and_an_absolute_path_without_control_characters() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let absolute = dir.0.path().to_str().unwrap().to_owned();
    // As the server receives it.
    let update = |agent: &str, home: String| -> UpdateRun {
        serde_json::from_value(serde_json::json!({"agent_homes": {agent: home}})).unwrap()
    };
    let rejected = [
        ("gemini", absolute.clone()),
        ("claude", "relative/home".to_owned()),
        (
            "claude",
            format!("{absolute}/{}", "a".repeat(MAX_PATH_BYTES)),
        ),
        ("claude", format!("{absolute}/a\u{7}b")),
    ];
    for (agent, home) in rejected {
        let error = store.update(&run.id, update(agent, home)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{agent}");
    }
    // The CLI refuses the same before sending.
    assert!(parse_update(&["--agent-home", "claude=relative/home"]).is_err());
    assert!(parse_update(&["--agent-home", "claude"]).is_err());
    let stored = store.get(&run.id).unwrap().unwrap();
    assert!(stored.agent_homes.is_empty());
    assert!(store.update(&run.id, update("copilot", absolute)).is_ok());
}

#[test]
fn update_never_changes_the_stage_and_rejects_a_stage_field() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let updated = store
        .update(&run.id, parse_update(&["--activity", "x"]).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(updated.stage, Stage::Define);
    let rejected: Result<UpdateRun, _> =
        serde_json::from_str(r#"{"stage":"build","status":"running"}"#);
    assert!(rejected.is_err());
}

#[test]
fn recording_the_mode_reports_the_workflow_skill_and_observed_skills_are_stored_once() {
    use crate::usage::token_usage::UsedSkill;
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let skills = |run: &Run| -> Vec<(String, Stage)> {
        run.run_reports
            .iter()
            .filter(|report| matches!(report.kind, ReportKind::Skill))
            .map(|report| (report.text.clone(), report.stage))
            .collect()
    };
    let mode = || parse_update(&["--mode", "jidoka-fast"]).unwrap();
    store.update(&run.id, mode()).unwrap();
    // Recording the mode again adds nothing.
    let after = store.update(&run.id, mode()).unwrap().unwrap();
    assert_eq!(skills(&after), vec![("jidoka-fast".into(), Stage::Define)]);

    let seen = |name: &str, stage| UsedSkill {
        stage,
        name: name.into(),
        at: 5,
    };
    let observed = [
        seen("jidoka-fast", Stage::Define),
        seen("rust", Stage::Build),
    ];
    store.observe_skills(&run.id, &observed).unwrap();
    store.observe_skills(&run.id, &observed).unwrap();
    let stored = store.get(&run.id).unwrap().unwrap();
    assert_eq!(
        skills(&stored),
        vec![
            ("jidoka-fast".into(), Stage::Define),
            ("rust".into(), Stage::Build)
        ]
    );
}

struct FixedGit;
impl crate::git::Git for FixedGit {
    fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
        Ok("abc1234".into())
    }
}

#[test]
fn findings_are_recorded_resolved_and_gate_the_rework() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let id = run.id.as_str();
    let update = |options: &[&str]| {
        let parsed = parse_update(options).map_err(io_invalid)?;
        store.update_with(id, parsed, &FixedGit)
    };
    update(&["--mode", "jidoka-balanced", "--worktree", "/work/wt"]).unwrap();
    advance_to(&store, id, Stage::Check);

    update(&["--finding", "code-review", "-", "Bug"]).unwrap();
    let rework =
        |finding: String, classification, decision: Option<&str>| LifecycleRequest::Rework {
            findings: vec![ClassifiedFinding {
                id: finding,
                classification,
            }],
            decision: decision.map(str::to_owned),
        };
    let start = || rework("F1".into(), FindingClassification::Behavior, None);
    assert!(
        store
            .transition(
                id,
                rework("F9".into(), FindingClassification::Behavior, None)
            )
            .is_err()
    );
    let moved = store.transition(id, start()).unwrap().unwrap();
    assert!(store.transition(id, start()).is_err());
    assert_eq!(moved.stage, Stage::Build);
    assert_eq!(moved.findings[0].id, "F1");
    assert_eq!(moved.findings[0].rework, Some(1));
    assert_eq!(moved.findings[0].reviewed_commit, "abc1234");

    let fixed = update(&["--finding-status", "F1=fixed"]).unwrap().unwrap();
    assert_eq!(fixed.findings[0].fix_commit.as_deref(), Some("abc1234"));
    assert!(fixed.findings[0].resolved_at.is_some());
    assert_eq!(
        moved.rework_history[0].included,
        ReworkFindings::Classified {
            findings: vec![ClassifiedFinding {
                id: "F1".into(),
                classification: FindingClassification::Behavior
            }]
        }
    );
    for (count, finding, decision) in [
        (2, "F2", None),
        (3, "F3", Some("User approved the behavior fix")),
        (4, "F4", Some("User approved one more Rework")),
    ] {
        advance_to(&store, id, Stage::Check);
        update(&["--finding", "code-review", "-", "Another bug"]).unwrap();
        if decision.is_some() {
            let saved = std::fs::read(dir.0.path().join("runs.json")).unwrap();
            let error = store
                .transition(
                    id,
                    rework(finding.into(), FindingClassification::Behavior, None),
                )
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("Rework {count} requires the user's decision")),
                "{error}"
            );
            assert_eq!(
                std::fs::read(dir.0.path().join("runs.json")).unwrap(),
                saved
            );
        }
        let moved = store
            .transition(
                id,
                rework(finding.into(), FindingClassification::Behavior, decision),
            )
            .unwrap()
            .unwrap();
        assert_eq!((moved.stage, moved.rework), (Stage::Build, Some(count)));
        let recorded = moved.rework_history.last().unwrap();
        assert!(
            matches!(&recorded.included, ReworkFindings::Classified { findings } if findings[0].id == finding)
        );
        assert_eq!(recorded.decision.as_deref(), decision);
        update(&["--finding-status", &format!("{finding}=fixed")]).unwrap();
    }
}

#[test]
fn a_rework_is_classified_by_every_open_finding_in_either_order() {
    for order in [["F1=wording", "F2=behavior"], ["F1=behavior", "F2=wording"]] {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = new_run(&store, "task");
        let id = run.id.as_str();
        let update = |options: &[&str]| {
            let parsed = parse_update(options).map_err(io_invalid)?;
            store.update_with(id, parsed, &FixedGit)
        };
        update(&["--mode", "jidoka-balanced", "--worktree", "/work/wt"]).unwrap();
        advance_to(&store, id, Stage::Check);
        update(&["--finding", "code-review", "-", "One"]).unwrap();
        update(&["--finding", "code-review", "-", "Two"]).unwrap();
        // Make this the third Rework so behavior needs the user's decision.
        for count in 1..=2 {
            let moved = store
                .transition(
                    id,
                    parse_lifecycle(
                        "rework",
                        &["--finding", "F1=wording", "--finding", "F2=wording"],
                    )
                    .unwrap(),
                )
                .unwrap()
                .unwrap();
            assert_eq!(moved.rework, Some(count));
            update(&[
                "--finding-status",
                "F1=fixed",
                "--finding-status",
                "F2=fixed",
            ])
            .unwrap();
            advance_to(&store, id, Stage::Check);
            update(&["--finding-status", "F1=open", "--finding-status", "F2=open"]).unwrap();
        }
        let saved = std::fs::read(dir.0.path().join("runs.json")).unwrap();
        let request = |extra: &[&str]| {
            let mut options = Vec::new();
            for finding in extra {
                options.extend(["--finding", finding]);
            }
            parse_lifecycle("rework", &options).unwrap()
        };
        // Missing, repeated, unknown and undecided requests change nothing.
        for (extra, wanted) in [
            (vec![order[0]], "Missing classifications"),
            (vec![order[0], order[0], order[1]], "more than once"),
            (vec![order[0], order[1], "F9=wording"], "not open"),
            (vec![order[0], order[1]], "requires the user's decision"),
        ] {
            let error = store.transition(id, request(&extra)).unwrap_err();
            assert!(error.to_string().contains(wanted), "{error}");
            assert_eq!(
                std::fs::read(dir.0.path().join("runs.json")).unwrap(),
                saved
            );
        }
        let mut with_decision = request(&order);
        if let LifecycleRequest::Rework { decision, .. } = &mut with_decision {
            *decision = Some("User approved the behavior fix".into());
        }
        let moved = store.transition(id, with_decision).unwrap().unwrap();
        assert_eq!((moved.stage, moved.rework), (Stage::Build, Some(3)));
        let recorded = moved.rework_history.last().unwrap();
        assert_eq!(recorded.classification, FindingClassification::Behavior);
        assert!(moved.findings.iter().all(|f| f.rework == Some(3)));
    }
}

#[test]
fn a_wording_only_rework_needs_a_decision_from_the_fourth() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let id = run.id.as_str();
    let update = |options: &[&str]| {
        let parsed = parse_update(options).map_err(io_invalid)?;
        store.update_with(id, parsed, &FixedGit)
    };
    update(&["--mode", "jidoka-balanced", "--worktree", "/work/wt"]).unwrap();
    advance_to(&store, id, Stage::Check);
    for count in 1..=4 {
        update(&["--finding", "code-review", "-", "Typo"]).unwrap();
        let finding = format!("F{count}=wording");
        let request = parse_lifecycle("rework", &["--finding", &finding]).unwrap();
        if count == 4 {
            let saved = std::fs::read(dir.0.path().join("runs.json")).unwrap();
            let error = store.transition(id, request).unwrap_err();
            assert!(
                error.to_string().contains("requires the user's decision"),
                "{error}"
            );
            assert_eq!(
                std::fs::read(dir.0.path().join("runs.json")).unwrap(),
                saved
            );
            break;
        }
        // Reworks 1 to 3 need no decision when every Finding is wording.
        let moved = store.transition(id, request).unwrap().unwrap();
        assert_eq!((moved.stage, moved.rework), (Stage::Build, Some(count)));
        update(&["--finding-status", &format!("F{count}=fixed")]).unwrap();
        advance_to(&store, id, Stage::Check);
    }
}

#[test]
fn failed_updates_leave_the_store_unchanged() {
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    let store = RunStore::open(&path).unwrap();
    let run = store
        .create(NewRun {
            title: "task".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            agent_homes: Default::default(),
            model_routing: None,
        })
        .unwrap();
    let apply = |stage, args: &[&str]| {
        advance_to(&store, &run.id, stage);
        store
            .update(&run.id, parse_update(args).unwrap())
            .unwrap()
            .unwrap()
    };
    let session = ["--session", "11111111-aaaa"];
    apply(Stage::Define, &["--activity", "working"]);
    apply(Stage::Build, &[]);
    apply(Stage::Build, &session);
    apply(Stage::Build, &session);
    apply(Stage::Finish, &["--summary", "finish=Completed"]);
    assert!(
        store
            .update("999999", UpdateRun::default())
            .unwrap()
            .is_none()
    );
    let before = std::fs::read(&path).unwrap();
    let invalid = UpdateRun {
        branch: FieldUpdate::Set("invalid branch".into()),
        ..UpdateRun::default()
    };
    assert!(store.update(&run.id, invalid).is_err());
    std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
    assert!(store.update(&run.id, UpdateRun::default()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(serde_json::to_vec(&store.list().unwrap()).unwrap(), before);
}

#[test]
fn concurrent_creates_keep_all_records_and_unique_ids() {
    let dir = TestDir::new();
    let store = std::sync::Arc::new(RunStore::open(&dir.0.path().join("runs.json")).unwrap());
    let threads: Vec<_> = (0..16)
        .map(|n| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .create(NewRun {
                        title: format!("run {n}"),
                        project: "/work".into(),
                        repository: None,
                        session: None,
                        agent_homes: Default::default(),
                        model_routing: None,
                    })
                    .unwrap()
            })
        })
        .collect();
    let created: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let ids: std::collections::HashSet<_> = created.iter().map(|run| run.id.clone()).collect();
    assert_eq!(ids.len(), 16);
    assert_eq!(
        load_runs(&dir.0.path().join("runs.json")).unwrap().len(),
        16
    );
}

#[test]
fn cli_update_options_preserve_set_clear_and_reject_conflicts() {
    let update = parse_update(&[]).unwrap();
    assert_eq!(serde_json::to_value(update).unwrap(), serde_json::json!({}));
    let update = parse_update(&[
        "--activity",
        "Implementing ticket",
        "--ticket-status",
        "https://example.com/42=in-progress",
    ])
    .unwrap();
    let value = serde_json::to_value(update).unwrap();
    assert_eq!(value["activity"], "Implementing ticket");
    assert_eq!(
        value["ticket_status"]["https://example.com/42"],
        "in-progress"
    );
    let update = parse_update(&["--clear-activity"]).unwrap();
    let value = serde_json::to_value(update).unwrap();
    assert_eq!(value.get("activity"), Some(&serde_json::Value::Null));
    for options in [
        vec!["--activity"],
        vec!["--activity", "--clear-activity"],
        vec!["--clear-activity", "--activity", "x"],
        vec!["--unknown"],
        vec!["--current-ticket", "file:///tmp/ticket"],
        vec!["--blocking-reason", ""],
    ] {
        assert!(parse_update(&options).is_err(), "options: {options:?}");
    }
}

#[test]
fn update_refuses_lifecycle_options_and_initialized_mode_changes() {
    for options in [
        &["--rework", "1"][..],
        &["--clear-rework"],
        &["--clear-mode"],
        &["running"],
    ] {
        assert!(parse_update(options).is_err(), "{options:?}");
    }
    for json in [
        r#"{"status":"running"}"#,
        r#"{"rework":1}"#,
        r#"{"blocking_reason":null}"#,
    ] {
        assert!(serde_json::from_str::<UpdateRun>(json).is_err(), "{json}");
    }
    let (_dir, store, id) = event_store();
    store
        .update(&id, parse_update(&["--mode", "balanced"]).unwrap())
        .unwrap();
    assert!(
        store
            .update(&id, parse_update(&["--mode", "full"]).unwrap())
            .is_err()
    );
    let update = serde_json::from_str(r#"{"mode":null}"#).unwrap();
    assert!(store.update(&id, update).is_err());
    assert_eq!(store.get(&id).unwrap().unwrap().mode, Some(Mode::Balanced));
}

/// A stored run for the unit tests of the update field groups, which need a `Run` but no store.
pub(super) fn sample_run() -> Run {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    new_run(&store, "task")
}

/// An open Finding with id `id`, for unit tests that need recorded Findings.
pub(super) fn finding(id: &str) -> Finding {
    Finding {
        id: id.into(),
        check: "security".into(),
        location: None,
        text: "Leak".into(),
        reviewed_commit: "abc1234".into(),
        rework: None,
        state: FindingState::Open,
        reason: None,
        fix_commit: None,
        at: 1,
        resolved_at: None,
    }
}

fn selection() -> crate::routing::ModelRouting {
    serde_json::from_value(serde_json::json!({
        "agent": "claude",
        "stages": {
            "build": {"model": "sonnet", "effort": "medium"},
            "check": {"model": "opus", "effort": "high"}
        }
    }))
    .unwrap()
}

fn routed_store(dir: &TestDir) -> (RunStore, Run) {
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = store
        .create(NewRun {
            title: "routed".into(),
            project: "/work/repo".into(),
            repository: None,
            session: None,
            agent_homes: Default::default(),
            model_routing: Some(selection()),
        })
        .unwrap();
    (store, run)
}

fn choice(routing: &ModelRouting, stage: Stage) -> (String, Option<String>) {
    let choice = routing.choice_for(stage).expect("a routed stage");
    (choice.model.clone(), choice.effort.clone())
}

fn pair(model: &str, effort: Option<&str>) -> (String, Option<String>) {
    (model.to_string(), effort.map(str::to_string))
}

#[test]
fn a_stage_override_beats_the_policy_replaces_earlier_ones_clears_and_survives_a_restart() {
    let dir = TestDir::new();
    let (store, run) = routed_store(&dir);
    let routing = |store: &RunStore| store.get(&run.id).unwrap().unwrap().model_routing.unwrap();
    assert_eq!(
        choice(&routing(&store), Stage::Check),
        pair("opus", Some("high"))
    );

    // An override with an effort, and one without that drops the policy's effort.
    store
        .update(
            &run.id,
            update(&[
                "--model-routing",
                "check=sonnet:low",
                "--model-routing",
                "define=haiku",
            ]),
        )
        .unwrap();
    assert_eq!(
        choice(&routing(&store), Stage::Check),
        pair("sonnet", Some("low"))
    );
    assert_eq!(choice(&routing(&store), Stage::Define), pair("haiku", None));
    assert_eq!(
        choice(&routing(&store), Stage::Build),
        pair("sonnet", Some("medium"))
    );
    // The policy's own stages are untouched.
    assert_eq!(routing(&store).stages[&Stage::Check].model, "opus");

    // A new set of overrides replaces the old one, and an unrelated update keeps it.
    store
        .update(&run.id, update(&["--model-routing", "build=opus"]))
        .unwrap();
    assert_eq!(
        choice(&routing(&store), Stage::Check),
        pair("opus", Some("high"))
    );
    assert_eq!(choice(&routing(&store), Stage::Build), pair("opus", None));
    store
        .update(&run.id, update(&["--activity", "later"]))
        .unwrap();
    assert_eq!(choice(&routing(&store), Stage::Build), pair("opus", None));

    let reopened = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    assert_eq!(
        choice(&routing(&reopened), Stage::Build),
        pair("opus", None)
    );

    store
        .update(&run.id, update(&["--clear-model-routing"]))
        .unwrap();
    assert_eq!(
        choice(&routing(&store), Stage::Build),
        pair("sonnet", Some("medium"))
    );
    assert!(routing(&store).overrides.is_empty());
}

#[test]
fn routing_history_survives_replacement_clear_and_restart_and_bounds_growth() {
    let dir = TestDir::new();
    let (store, run) = routed_store(&dir);
    for name in ["opus", "haiku"] {
        store
            .update(
                &run.id,
                update(&["--model-routing", &format!("build={name}")]),
            )
            .unwrap();
    }
    store
        .update(&run.id, update(&["--clear-model-routing"]))
        .unwrap();
    let reopened = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let routing = reopened
        .get(&run.id)
        .unwrap()
        .unwrap()
        .model_routing
        .unwrap();
    for model in ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"] {
        assert!(routing.allows_model(Stage::Build, model), "{model}");
        assert!(routing.allows_model(Stage::Finish, model), "{model}");
    }
    assert!(!routing.allows_model(Stage::Build, "unrelated"));
    for n in 0..30 {
        store
            .update(
                &run.id,
                update(&["--model-routing", &format!("build=model{n}")]),
            )
            .unwrap();
    }
    let error = store
        .update(&run.id, update(&["--model-routing", "build=overflow"]))
        .unwrap_err();
    assert!(
        error.to_string().contains("at most 32 prior model choices"),
        "{error}"
    );
}

#[test]
fn a_model_routing_selection_recorded_at_start_survives_updates_and_a_restart() {
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    let store = RunStore::open(&path).unwrap();
    let run = store
        .create(NewRun {
            title: "routed".into(),
            project: "/work/repo".into(),
            repository: None,
            session: None,
            agent_homes: Default::default(),
            model_routing: Some(selection()),
        })
        .unwrap();
    assert_eq!(run.model_routing, Some(selection()));
    let run = store
        .update(&run.id, update(&["--activity", "later"]))
        .unwrap()
        .unwrap();
    assert_eq!(run.model_routing, Some(selection()));
    let reopened = RunStore::open(&path).unwrap();
    let run = reopened.get(&run.id).unwrap().unwrap();
    assert_eq!(run.model_routing, Some(selection()));
}

fn update(options: &[&str]) -> UpdateRun {
    parse_update(options).unwrap()
}

#[test]
fn context_is_set_kept_and_cleared_one_field_at_a_time() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    assert_eq!(run.repository.as_deref(), Some("/work/repo"));
    assert!(run.worktree.is_none() && run.tickets.is_empty() && run.summaries.is_empty());

    let run = store
        .update(
            &run.id,
            update(&[
                "--mode",
                "jidoka-fast",
                "--worktree",
                "/work/repo.worktrees/feature",
                "--branch",
                "feature/x",
                "--base-commit",
                "ABCDEF1234",
                "--spec",
                "https://github.com/o/r/issues/1",
                "--pr",
                "https://github.com/o/r/pull/2",
                "--summary",
                "define=Spec written",
            ]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(run.mode, Some(Mode::Fast));
    assert_eq!(run.branch.as_deref(), Some("feature/x"));
    assert_eq!(
        run.base_commit.as_deref(),
        Some("abcdef1234"),
        "stored in lowercase"
    );
    assert_eq!(run.summaries[&Stage::Define], "Spec written");

    // An update that mentions none of the context keeps all of it.
    let kept = store
        .update(&run.id, update(&["--activity", "Working"]))
        .unwrap()
        .unwrap();
    assert_eq!(kept.worktree, run.worktree);
    assert_eq!(kept.spec, run.spec);
    assert_eq!(kept.pr, run.pr);
    assert_eq!(kept.summaries, run.summaries);

    // Each clear flag removes only its own field.
    let cleared = store
        .update(
            &run.id,
            update(&["--clear-pr", "--clear-summary", "define"]),
        )
        .unwrap()
        .unwrap();
    assert!(
        cleared.pr.is_none() && cleared.mode == Some(Mode::Fast) && cleared.summaries.is_empty()
    );
    assert_eq!(cleared.branch.as_deref(), Some("feature/x"));
    assert_eq!(cleared.spec, run.spec);
}

#[test]
fn stage_summaries_survive_later_stages() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    store
        .update(&run.id, update(&["--summary", "define=Planned"]))
        .unwrap();
    store
        .update(
            &run.id,
            parse_update(&["--summary", "build=Built three tickets"]).unwrap(),
        )
        .unwrap();
    let run = store
        .update(
            &run.id,
            parse_update(&["--summary", "define=Planned again"]).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(run.summaries[&Stage::Define], "Planned again");
    assert_eq!(run.summaries[&Stage::Build], "Built three tickets");
}

#[test]
fn the_ticket_list_is_ordered_and_keeps_progress_when_relisted() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let run = store
        .update(
            &run.id,
            update(&[
                "--ticket",
                "https://x.test/1 First one",
                "--ticket",
                "docs/t2.md Second",
                "--ticket-status",
                "https://x.test/1=in-progress",
            ]),
        )
        .unwrap()
        .unwrap();
    let view = |run: &Run| {
        run.tickets
            .iter()
            .map(|t| (t.reference.clone(), t.status))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        view(&run),
        [
            ("https://x.test/1".into(), TicketStatus::InProgress),
            ("docs/t2.md".into(), TicketStatus::Pending)
        ]
    );
    // Listing again in a new order with a new ticket keeps the status of the kept ones.
    let run = store
        .update(
            &run.id,
            update(&[
                "--ticket",
                "docs/t2.md Second",
                "--ticket",
                "https://x.test/1 First one",
                "--ticket",
                "docs/t3.md Third",
            ]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        view(&run),
        [
            ("docs/t2.md".into(), TicketStatus::Pending),
            ("https://x.test/1".into(), TicketStatus::InProgress),
            ("docs/t3.md".into(), TicketStatus::Pending),
        ]
    );
    let run = store
        .update(
            &run.id,
            update(&[
                "--ticket-status",
                "https://x.test/1=done",
                "--ticket-status",
                "docs/t2.md=in-progress",
            ]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(run.tickets[0].status, TicketStatus::InProgress);
    assert_eq!(run.tickets[1].status, TicketStatus::Done);
    let run = store
        .update(&run.id, update(&["--clear-tickets"]))
        .unwrap()
        .unwrap();
    assert!(run.tickets.is_empty());
}

#[test]
fn inconsistent_updates_are_rejected_with_a_reason_and_change_nothing() {
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    let store = RunStore::open(&path).unwrap();
    let run = new_run(&store, "task");
    store
        .update(&run.id, update(&["--ticket", "a A", "--ticket", "b B"]))
        .unwrap();
    let before = fs::read(&path).unwrap();

    for (options, reason) in [
        (
            vec!["--ticket-status", "missing=done"],
            "No ticket missing is listed",
        ),
        (
            vec![
                "--ticket-status",
                "a=in-progress",
                "--ticket-status",
                "b=in-progress",
            ],
            "Only one ticket",
        ),
        // Even with a valid change beside it, nothing is applied.
        (
            vec!["--branch", "other", "--ticket-status", "zzz=done"],
            "No ticket zzz",
        ),
    ] {
        let error = store.update(&run.id, update(&options)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains(reason), "{error}");
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(store.get(&run.id).unwrap().unwrap().branch.is_none());
}

#[test]
fn a_mark_records_the_activity_of_the_update_that_created_it() {
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    let store = RunStore::open(&path).unwrap();
    let run = new_run(&store, "task");
    store
        .update(&run.id, parse_update(&["--mode", "balanced"]).unwrap())
        .unwrap();
    assert_eq!(run.timeline[0].activity, None);
    let building = force_advance(&store, &run.id, Some("Writing the code"));
    assert_eq!(
        building.timeline[1].activity.as_deref(),
        Some("Writing the code")
    );
    // An update that adds no mark leaves the pass's activity alone.
    let same = store
        .update(
            &run.id,
            parse_update(&["--activity", "Still going"]).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(same.timeline.len(), 2);
    assert_eq!(
        same.timeline[1].activity.as_deref(),
        Some("Writing the code")
    );
    // An update that makes a mark, here by starting a ticket, records the activity given with it.
    store
        .update(
            &run.id,
            parse_update(&["--ticket", "https://example.com/1 One"]).unwrap(),
        )
        .unwrap();
    let ticket = store
        .update(
            &run.id,
            parse_update(&[
                "--ticket-status",
                "https://example.com/1=in-progress",
                "--activity",
                "Writing ticket one",
            ])
            .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        ticket.timeline[2].activity.as_deref(),
        Some("Writing ticket one")
    );
    // A mark made by an advance without an activity does not borrow the previous one.
    let checking = force_advance(&store, &run.id, None);
    assert_eq!(checking.stage, Stage::Check);
    assert_eq!(checking.timeline[3].activity, None);

    // The activity survives a reload from disk, and shows in the serialised run.
    let reloaded = RunStore::open(&path)
        .unwrap()
        .get(&run.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded.timeline[1].activity.as_deref(),
        Some("Writing the code")
    );
    let json = serde_json::to_value(&reloaded).unwrap();
    assert_eq!(json["timeline"][1]["activity"], "Writing the code");
    assert!(json["timeline"][3].get("activity").is_none());
}

#[test]
fn finishing_a_run_closes_its_window_and_an_update_cannot_reopen_it() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    store
        .update(
            &run.id,
            parse_update(&["--mode", "fast", "--summary", "finish=Completed"]).unwrap(),
        )
        .unwrap();
    advance_to(&store, &run.id, Stage::Finish);
    let done = force_advance(&store, &run.id, None);
    assert!(done.finished_at.is_some());
    let update = store
        .update(
            &run.id,
            parse_update(&["--activity", "Worktree cleaned up"]).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(update.status, RunStatus::Done);
    assert_eq!(update.finished_at, done.finished_at);
    assert_eq!(update.completion, done.completion);
    assert!(
        store
            .transition(
                &run.id,
                LifecycleRequest::Resume {
                    source: "retry".into()
                }
            )
            .is_err()
    );
    assert!(
        store
            .transition(
                &run.id,
                LifecycleRequest::Block {
                    reason: "retry".into()
                }
            )
            .is_err()
    );
}

#[test]
fn ticket_reports_reject_bad_values_and_unknown_tickets_without_saving() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let created = new_run(&store, "task");
    for args in [
        vec!["--ticket-report", "missing"],
        vec!["--ticket-report", "t1", "bad", "x"],
        vec!["--ticket-report", "t1", "skill", ""],
        vec!["--ticket-report", "t1", "activity", "bad\ntext"],
    ] {
        assert!(parse_update(&args).is_err(), "{args:?}");
    }
    let update = parse_update(&["--ticket-report", "t1", "skill", "jidoka-tdd"]).unwrap();
    assert!(store.update(&created.id, update).is_err());
    assert!(
        store
            .get(&created.id)
            .unwrap()
            .unwrap()
            .ticket_reports
            .is_empty()
    );
}

#[test]
fn sessions_are_recorded_once_validated_and_kept_across_updates() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let created = store
        .create(NewRun {
            title: "t".into(),
            project: "/p".into(),
            repository: None,
            session: Some("11111111-aaaa".into()),
            agent_homes: Default::default(),
            model_routing: None,
        })
        .unwrap();
    assert_eq!(created.sessions, ["11111111-aaaa"]);
    let run = store
        .update(
            &created.id,
            parse_update(&[
                "--session",
                "22222222-bbbb",
                "--session",
                "11111111-aaaa",
                "--session",
                "22222222-bbbb",
            ])
            .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(run.sessions, ["11111111-aaaa", "22222222-bbbb"]);
    let kept = store
        .update(&created.id, update(&["--activity", "x"]))
        .unwrap()
        .unwrap();
    assert_eq!(kept.sessions, run.sessions);
    for bad in [
        "short",
        "has space in it",
        "../../etc/passwd",
        "semi;colon-id",
        "",
    ] {
        assert!(parse_update(&["--session", bad]).is_err(), "{bad}");
    }
}

#[test]
fn usage_is_replaced_atomically_and_a_failed_save_keeps_the_old_usage() {
    use crate::usage::token_usage::{Usage, UsageStatus};
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    let store = RunStore::open(&path).unwrap();
    let run = new_run(&store, "task");
    assert!(run.usage.is_none());
    store
        .set_usage(&run.id, Usage::not_tracked("first", 1))
        .unwrap();
    fs::create_dir(path.with_extension("json.tmp")).unwrap();
    assert!(
        store
            .set_usage(&run.id, Usage::not_tracked("second", 2))
            .is_err()
    );
    assert_eq!(
        store
            .get(&run.id)
            .unwrap()
            .unwrap()
            .usage
            .unwrap()
            .reason
            .as_deref(),
        Some("first")
    );
    fs::remove_dir(path.with_extension("json.tmp")).unwrap();
    store
        .set_usage(&run.id, Usage::not_tracked("third", 3))
        .unwrap();
    let restarted = RunStore::open(&path).unwrap();
    let usage = restarted.get(&run.id).unwrap().unwrap().usage.unwrap();
    assert_eq!(
        (usage.status, usage.reason.as_deref()),
        (UsageStatus::NotTracked, Some("third"))
    );
    // An unknown run is left alone.
    store.set_usage("nope", Usage::not_tracked("x", 1)).unwrap();
}

#[test]
fn only_subagent_usage_on_another_model_warns_when_a_routed_stage_is_done() {
    use crate::usage::token_usage::Usage;
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    let store = RunStore::open(&path).unwrap();
    let run = new_run(&store, "task");
    let id = run.id.clone();
    let mut runs = store.list().unwrap();
    let mut stored = runs.remove(0);
    stored.status = RunStatus::Done;
    stored.model_routing = serde_json::from_value(
        serde_json::json!({"agent": "claude", "stages": {"define": {"model": "sonnet"}}}),
    )
    .unwrap();
    fs::write(&path, serde_json::to_string(&[stored]).unwrap()).unwrap();
    let store = RunStore::open(&path).unwrap();
    let usage = |subagents: bool| -> Usage {
        let mut row = serde_json::json!({"stage": "define", "model": "claude-opus-5-5", "input": 10, "output": 5});
        if subagents {
            row["subagents"] = serde_json::json!({"input": 4, "output": 2});
        }
        serde_json::from_value(
            serde_json::json!({"status": "tracked", "collected_at": 1, "rows": [row]}),
        )
        .unwrap()
    };
    store.set_usage(&id, usage(false)).unwrap();
    assert!(store.get(&id).unwrap().unwrap().run_reports.is_empty());
    store.set_usage(&id, usage(true)).unwrap();
    let reports = store.get(&id).unwrap().unwrap().run_reports;
    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].text.ends_with(
            "define subagent usage recorded model claude-opus-5-5 while sonnet was assigned"
        ),
        "{}",
        reports[0].text
    );
    let mut row =
        serde_json::json!({"stage": "define", "model": "sonnet", "input": 10, "output": 5});
    row["subagents"] = serde_json::json!({"input": 4, "output": 2});
    let on_model: Usage = serde_json::from_value(
        serde_json::json!({"status": "tracked", "collected_at": 2, "rows": [row]}),
    )
    .unwrap();
    store.set_usage(&id, on_model).unwrap();
    let after = store.get(&id).unwrap().unwrap().run_reports;
    assert_eq!(after.len(), 1, "{after:?}");
}

fn event_store() -> (TestDir, RunStore, String) {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = store
        .create(NewRun {
            title: "task".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            agent_homes: Default::default(),
            model_routing: None,
        })
        .unwrap();
    (dir, store, run.id)
}

#[test]
fn workspace_retry_resumes_its_own_block() {
    let (dir, store, id) = event_store();
    let ready = || LifecycleRequest::WorkspaceReady {
        update: Box::new(parse_update(&["--worktree", "/work/feature"]).unwrap()),
    };
    let blocked = store
        .transition(
            &id,
            LifecycleRequest::WorkspaceBlocked {
                reason: "Missing origin".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(blocked.status, RunStatus::Blocked);
    let saved = std::fs::read(dir.0.path().join("runs.json")).unwrap();
    let incomplete = LifecycleRequest::WorkspaceReady {
        update: Box::default(),
    };
    let error = store.transition(&id, incomplete).unwrap_err();
    assert_eq!(error.to_string(), "Workspace ready requires a worktree");
    assert_eq!(
        std::fs::read(dir.0.path().join("runs.json")).unwrap(),
        saved
    );
    let resumed = store.transition(&id, ready()).unwrap().unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    assert_eq!(resumed.worktree.as_deref(), Some("/work/feature"));
    assert_eq!(resumed.events[1].source, Some(LifecycleSource::Workspace));

    let other = new_run(&store, "Manually blocked");
    store
        .transition(
            &other.id,
            LifecycleRequest::Block {
                reason: "User decision".into(),
            },
        )
        .unwrap();
    let manual = store.transition(&other.id, ready()).unwrap().unwrap();
    assert_eq!(manual.status, RunStatus::Blocked);
    assert_eq!(manual.blocking_reason.as_deref(), Some("User decision"));
}

#[test]
fn blocking_and_resuming_record_events_without_touching_the_timeline() {
    let (_dir, store, id) = event_store();
    let block = || LifecycleRequest::Block {
        reason: "Need keys".into(),
    };
    let resume = || LifecycleRequest::Resume {
        source: "User supplied credentials".into(),
    };
    advance_to(&store, &id, Stage::Build);
    assert!(store.transition(&id, resume()).is_err());
    let blocked = store.transition(&id, block()).unwrap().unwrap();
    assert_eq!(blocked.events.len(), 1);
    assert_eq!(blocked.events[0].kind, RunEventKind::Blocked);
    assert_eq!(blocked.events[0].stage, Stage::Build);
    assert_eq!(blocked.events[0].reason.as_deref(), Some("Need keys"));
    assert_eq!(blocked.events[0].at, blocked.updated_at.unwrap());
    assert!(store.transition(&id, block()).is_err());
    let again = store
        .update(
            &id,
            parse_update(&["--activity", "Planning next step"]).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(again.status, RunStatus::Blocked);
    assert_eq!(again.blocking_reason, blocked.blocking_reason);
    assert_eq!(again.events, blocked.events);
    let resumed = store.transition(&id, resume()).unwrap().unwrap();
    assert_eq!(resumed.events.len(), 2);
    assert_eq!(resumed.events[1].kind, RunEventKind::Resumed);
    assert_eq!(resumed.events[1].stage, Stage::Build);
    assert_eq!(
        resumed.events[1].reason.as_deref(),
        Some("User supplied credentials")
    );
    assert!(resumed.events[0].at <= resumed.events[1].at);
    // Events are not marks: only the stage changes make marks.
    advance_to(&store, &id, Stage::Check);
    let resumed = store.get(&id).unwrap().unwrap();
    assert_eq!(resumed.events.len(), 2);
    let stages: Vec<_> = resumed.timeline.iter().map(|m| m.stage).collect();
    assert_eq!(stages, [Stage::Define, Stage::Build, Stage::Check]);
    let json = serde_json::to_value(&resumed).unwrap();
    assert_eq!(json["events"][0]["kind"], "blocked");
    assert_eq!(json["events"][1]["kind"], "resumed");
}

#[test]
fn an_advance_is_refused_when_the_agent_changes_the_run_during_its_check() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    store
        .update(&run.id, parse_update(&["--mode", "fast"]).unwrap())
        .unwrap();
    let error = store
        .advance(&run.id, AdvanceMetadata::default(), |_| {
            store
                .transition(
                    &run.id,
                    LifecycleRequest::Block {
                        reason: "waiting".into(),
                    },
                )
                .unwrap();
            Ok(Exit::Override {
                reason: "Spec agreed by hand".into(),
            })
        })
        .unwrap_err();
    assert!(
        error.contains("changed while checking"),
        "unexpected error: {error}"
    );
    let current = store.get(&run.id).unwrap().unwrap();
    assert_eq!(current.stage, Stage::Define);
    assert_eq!(current.status, RunStatus::Blocked);
}

#[test]
fn active_ticket_accepts_local_references() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let updated = store
        .update(
            &run.id,
            parse_update(&[
                "--ticket",
                "/tmp/ticket.md Local ticket",
                "--ticket-status",
                "/tmp/ticket.md=in-progress",
            ])
            .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.active_ticket().unwrap().reference, "/tmp/ticket.md");
}

#[test]
fn mode_aliases_store_canonical_names() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    for (alias, canonical) in [
        ("fast", "jidoka-fast"),
        ("balanced", "jidoka-balanced"),
        ("full", "jidoka-full"),
    ] {
        assert!(
            crate::run::begin::parse_options("task", &["--mode", alias]).is_ok(),
            "mode alias {alias} should be accepted"
        );
        let run = new_run(&store, "task");
        let updated = store
            .update(&run.id, parse_update(&["--mode", alias]).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::to_value(updated).unwrap()["mode"], canonical);
    }
}

#[test]
fn json_null_clears_and_missing_keeps_each_field_group() {
    let cleared: UpdateRun =
        serde_json::from_str(r#"{"tickets":null,"model_routing":null,"summaries":{"build":null}}"#)
            .unwrap();
    assert!(matches!(cleared.tickets.tickets, FieldUpdate::Clear));
    assert!(matches!(cleared.routing.model_routing, FieldUpdate::Clear));
    let kept: UpdateRun = serde_json::from_str("{}").unwrap();
    assert!(kept.tickets.tickets.is_keep() && kept.routing.model_routing.is_keep());
    assert_eq!(
        serde_json::to_value(&cleared).unwrap()["summaries"],
        serde_json::json!({"build": null})
    );
}

#[test]
fn update_accepts_free_text_that_starts_with_dashes() {
    let update = parse_update(&[
        "--activity",
        "--rework is odd",
        "--finding",
        "code-review",
        "-",
        "--x and --y",
    ])
    .unwrap();
    assert!(matches!(update.activity, FieldUpdate::Set(ref text) if text == "--rework is odd"));

    let error = parse_update(&["--activity", "--summary"]).unwrap_err();
    assert!(error.contains("found --summary, which looks like an option"));
    assert!(parse_update(&["--activity"]).is_err());
}
