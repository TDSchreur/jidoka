use super::*;
use crate::test_support::TestDir;

const FINISH_SUMMARY: [&str; 2] = ["--summary", "finish=Shipped"];

#[test]
fn finish_done_needs_a_finish_summary() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let before = store.get(&run.id).unwrap().unwrap();
    let error = store
        .update(&run.id, parse_update("finish", "done", &[]).unwrap())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("--summary finish="));
    let after = store.get(&run.id).unwrap().unwrap();
    assert_eq!(after.stage, before.stage);
    assert_eq!(after.status, before.status);
    assert!(after.finished_at.is_none() && after.summaries.is_empty());

    // Other stages are not affected; a summary in the same update is enough.
    store
        .update(&run.id, parse_update("check", "done", &[]).unwrap())
        .unwrap();
    let done = store
        .update(
            &run.id,
            parse_update("finish", "done", &FINISH_SUMMARY).unwrap(),
        )
        .unwrap()
        .unwrap()
        .run;
    assert!(matches!(done.status, RunStatus::Done));
    // A summary recorded earlier counts.
    let again = store
        .update(&run.id, parse_update("finish", "done", &[]).unwrap())
        .unwrap();
    assert!(again.is_some());
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
            model_routing: None,
        })
        .unwrap();
    let apply = |stage, status, args: &[&str]| {
        store
            .update(&run.id, parse_update(stage, status, args).unwrap())
            .unwrap()
            .unwrap()
    };
    let session = ["--session", "11111111-aaaa"];
    apply("define", "running", &["--activity", "working"]);
    apply("build", "running", &[]);
    apply("build", "running", &session);
    apply("build", "running", &session);
    apply("build", "done", &[]);
    apply("build", "done", &[]);
    apply("finish", "done", &FINISH_SUMMARY);
    apply("finish", "done", &FINISH_SUMMARY);
    assert!(
        store
            .update("999999", UpdateRun::new(Stage::Build, RunStatus::Running))
            .unwrap()
            .is_none()
    );
    let before = std::fs::read(&path).unwrap();
    let mut invalid = UpdateRun::new(Stage::Check, RunStatus::Running);
    invalid.branch = FieldUpdate::Set("invalid branch".into());
    assert!(store.update(&run.id, invalid).is_err());
    std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
    assert!(
        store
            .update(&run.id, UpdateRun::new(Stage::Check, RunStatus::Running))
            .is_err()
    );
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
    let update = parse_update("build", "running", &[]).unwrap();
    assert_eq!(
        serde_json::to_value(update).unwrap(),
        serde_json::json!({"stage":"build","status":"running"})
    );
    let update = parse_update(
        "build",
        "blocked",
        &[
            "--activity",
            "Implementing ticket",
            "--current-ticket",
            "https://example.com/42",
            "--blocking-reason",
            "Need input",
        ],
    )
    .unwrap();
    let value = serde_json::to_value(update).unwrap();
    assert_eq!(value["activity"], "Implementing ticket");
    assert_eq!(value["current_ticket"], "https://example.com/42");
    assert_eq!(value["blocking_reason"], "Need input");
    let update = parse_update(
        "finish",
        "done",
        &[
            "--clear-activity",
            "--clear-current-ticket",
            "--clear-blocking-reason",
        ],
    )
    .unwrap();
    let value = serde_json::to_value(update).unwrap();
    for field in ["activity", "current_ticket", "blocking_reason"] {
        assert_eq!(value.get(field), Some(&serde_json::Value::Null));
    }
    for options in [
        vec!["--activity"],
        vec!["--activity", "--clear-activity"],
        vec!["--clear-activity", "--activity", "x"],
        vec!["--clear-current-ticket", "--clear-current-ticket"],
        vec!["--unknown"],
        vec!["--current-ticket", "file:///tmp/ticket"],
        vec!["--blocking-reason", ""],
    ] {
        assert!(
            parse_update("build", "running", &options).is_err(),
            "options: {options:?}"
        );
    }
    assert!(parse_update("unknown", "running", &[]).is_err());
    assert!(parse_update("build", "unknown", &[]).is_err());
}

#[test]
fn cli_rework_option_sets_clears_and_rejects_invalid_counts() {
    let update = parse_update("build", "running", &["--rework", "2"]).unwrap();
    assert_eq!(serde_json::to_value(update).unwrap()["rework"], 2);
    let update = parse_update("check", "running", &["--clear-rework"]).unwrap();
    assert_eq!(
        serde_json::to_value(update).unwrap().get("rework"),
        Some(&serde_json::Value::Null)
    );
    for options in [
        vec!["--rework"],
        vec!["--rework", "0"],
        vec!["--rework", "-1"],
        vec!["--rework", "two"],
        vec!["--rework", "1.5"],
        vec!["--rework", "100"],
        vec!["--rework", "1", "--clear-rework"],
        vec!["--rework", "1", "--rework", "2"],
    ] {
        assert!(
            parse_update("build", "running", &options).is_err(),
            "options: {options:?}"
        );
    }
    let range = format!("1 to {MAX_REWORK}");
    for count in ["0", "100", "two"] {
        let Err(error) = parse_update("build", "running", &["--rework", count]) else {
            panic!("--rework {count} should be rejected");
        };
        assert!(error.to_string().contains(&range), "{error}");
    }
    assert!(UPDATE_HELP.contains(&format!("--rework <1-{MAX_REWORK}>")));
}

fn new_run(store: &RunStore, title: &str) -> Run {
    store
        .create(NewRun {
            title: title.into(),
            project: "/work/repo.worktrees/feature".into(),
            repository: Some("/work/repo".into()),
            session: None,
            model_routing: None,
        })
        .unwrap()
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
            model_routing: Some(selection()),
        })
        .unwrap();
    assert_eq!(run.model_routing, Some(selection()));
    let run = store
        .update(&run.id, update(&["--activity", "later"]))
        .unwrap()
        .unwrap()
        .run;
    assert_eq!(run.model_routing, Some(selection()));
    let reopened = RunStore::open(&path).unwrap();
    let run = reopened.get(&run.id).unwrap().unwrap();
    assert_eq!(run.model_routing, Some(selection()));
}

fn update(options: &[&str]) -> UpdateRun {
    parse_update("build", "running", options).unwrap()
}

fn warnings_for(
    store: &RunStore,
    id: &str,
    stage: &str,
    status: &str,
    options: &[&str],
) -> Vec<String> {
    store
        .update(id, parse_update(stage, status, options).unwrap())
        .unwrap()
        .unwrap()
        .warnings
}

#[test]
fn done_on_a_never_entered_stage_warns_but_the_normal_flow_does_not() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    assert!(warnings_for(&store, &run.id, "build", "running", &[]).is_empty());
    assert!(warnings_for(&store, &run.id, "build", "done", &[]).is_empty());
    let warnings = warnings_for(&store, &run.id, "finish", "done", &FINISH_SUMMARY);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("finish") && warnings[0].contains("never entered"));
    assert_eq!(store.get(&run.id).unwrap().unwrap().stage, Stage::Finish);
}

#[test]
fn a_full_run_without_a_spec_warns_when_build_is_done_or_check_starts() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let id = &run.id;
    warnings_for(&store, id, "build", "running", &["--mode", "jidoka-full"]);
    let done = warnings_for(&store, id, "build", "done", &[]);
    assert_eq!(done.len(), 1);
    assert!(done[0].contains("no spec"));
    assert!(!done[0].contains("before Build"), "{}", done[0]);
    let check = warnings_for(&store, id, "check", "running", &[]);
    assert_eq!(check.len(), 1);
    assert!(warnings_for(&store, id, "check", "running", &[]).is_empty());
    warnings_for(
        &store,
        id,
        "build",
        "running",
        &["--spec", "https://example.com/1"],
    );
    assert!(warnings_for(&store, id, "build", "done", &[]).is_empty());
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
        .unwrap()
        .run;
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
        .unwrap()
        .run;
    assert_eq!(kept.worktree, run.worktree);
    assert_eq!(kept.spec, run.spec);
    assert_eq!(kept.pr, run.pr);
    assert_eq!(kept.summaries, run.summaries);

    // Each clear flag removes only its own field.
    let cleared = store
        .update(
            &run.id,
            update(&["--clear-pr", "--clear-mode", "--clear-summary", "define"]),
        )
        .unwrap()
        .unwrap()
        .run;
    assert!(cleared.pr.is_none() && cleared.mode.is_none() && cleared.summaries.is_empty());
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
            parse_update(
                "check",
                "running",
                &["--summary", "build=Built three tickets"],
            )
            .unwrap(),
        )
        .unwrap();
    let run = store
        .update(
            &run.id,
            parse_update("finish", "running", &["--summary", "define=Planned again"]).unwrap(),
        )
        .unwrap()
        .unwrap()
        .run;
    assert!(matches!(run.stage, Stage::Finish));
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
        .unwrap()
        .run;
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
        .unwrap()
        .run;
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
        .unwrap()
        .run;
    assert_eq!(run.tickets[0].status, TicketStatus::InProgress);
    assert_eq!(run.tickets[1].status, TicketStatus::Done);
    let run = store
        .update(&run.id, update(&["--clear-tickets"]))
        .unwrap()
        .unwrap()
        .run;
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
    let after = |options: &[&str], stage: &str| {
        store
            .update(&run.id, parse_update(stage, "running", options).unwrap())
            .unwrap()
            .unwrap()
            .run
    };
    assert_eq!(run.timeline[0].activity, None);
    let building = after(&["--activity", "Writing the code"], "build");
    assert_eq!(
        building.timeline[1].activity.as_deref(),
        Some("Writing the code")
    );
    // An update that adds no mark leaves the pass's activity alone.
    let same = after(&["--activity", "Still going"], "build");
    assert_eq!(same.timeline.len(), 2);
    assert_eq!(
        same.timeline[1].activity.as_deref(),
        Some("Writing the code")
    );
    // A mark made by an update without an activity does not borrow the previous one.
    let checking = after(&[], "check");
    assert_eq!(checking.timeline[2].activity, None);

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
    assert!(json["timeline"][2].get("activity").is_none());
}

#[test]
fn finishing_a_run_closes_its_window_and_reopening_clears_it() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let done = store
        .update(
            &run.id,
            parse_update("finish", "done", &FINISH_SUMMARY).unwrap(),
        )
        .unwrap()
        .unwrap()
        .run;
    let end = done.finished_at.expect("closed when done");
    let again = store
        .update(
            &run.id,
            parse_update(
                "finish",
                "done",
                &["--summary", "finish=Shipped", "--activity", "x"],
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap()
        .run;
    assert_eq!(
        again.finished_at,
        Some(end),
        "a second done update keeps the first end"
    );
    let reopened = store
        .update(&run.id, parse_update("build", "running", &[]).unwrap())
        .unwrap()
        .unwrap()
        .run;
    assert_eq!(reopened.finished_at, None);
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
        assert!(parse_update("build", "running", &args).is_err(), "{args:?}");
    }
    let update = parse_update(
        "build",
        "running",
        &["--ticket-report", "t1", "skill", "jidoka-tdd"],
    )
    .unwrap();
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
            model_routing: None,
        })
        .unwrap();
    assert_eq!(created.sessions, ["11111111-aaaa"]);
    let run = store
        .update(
            &created.id,
            parse_update(
                "build",
                "running",
                &[
                    "--session",
                    "22222222-bbbb",
                    "--session",
                    "11111111-aaaa",
                    "--session",
                    "22222222-bbbb",
                ],
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap()
        .run;
    assert_eq!(run.sessions, ["11111111-aaaa", "22222222-bbbb"]);
    let kept = store
        .update(&created.id, update(&["--activity", "x"]))
        .unwrap()
        .unwrap()
        .run;
    assert_eq!(kept.sessions, run.sessions);
    for bad in [
        "short",
        "has space in it",
        "../../etc/passwd",
        "semi;colon-id",
        "",
    ] {
        assert!(
            parse_update("build", "running", &["--session", bad]).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn usage_is_replaced_atomically_and_a_failed_save_keeps_the_old_usage() {
    use crate::usage::{Usage, UsageStatus};
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

fn event_store() -> (TestDir, RunStore, String) {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = store
        .create(NewRun {
            title: "task".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        })
        .unwrap();
    (dir, store, run.id)
}

#[test]
fn blocking_and_resuming_record_events_without_touching_the_timeline() {
    let (_dir, store, id) = event_store();
    let apply = |stage, status, args: &[&str]| {
        store
            .update(&id, parse_update(stage, status, args).unwrap())
            .unwrap()
            .unwrap()
            .run
    };
    let running = apply("build", "running", &[]);
    assert!(running.events.is_empty());
    let blocked = apply("build", "blocked", &["--blocking-reason", "Need keys"]);
    assert_eq!(blocked.events.len(), 1);
    assert_eq!(blocked.events[0].kind, RunEventKind::Blocked);
    assert_eq!(blocked.events[0].stage, Stage::Build);
    assert_eq!(blocked.events[0].reason.as_deref(), Some("Need keys"));
    assert_eq!(blocked.events[0].at, blocked.updated_at.unwrap());
    // Blocked again, even with a new reason, adds nothing.
    let again = apply(
        "build",
        "blocked",
        &["--blocking-reason", "Still need keys"],
    );
    assert_eq!(again.events.len(), 1);
    let resumed = apply("check", "running", &["--clear-blocking-reason"]);
    assert_eq!(resumed.events.len(), 2);
    assert_eq!(resumed.events[1].kind, RunEventKind::Resumed);
    assert_eq!(resumed.events[1].stage, Stage::Check);
    assert!(resumed.events[1].reason.is_none());
    assert!(resumed.events[0].at <= resumed.events[1].at);
    // Events are not marks: only the stage changes make marks.
    let stages: Vec<_> = resumed.timeline.iter().map(|m| m.stage).collect();
    assert_eq!(stages, [Stage::Define, Stage::Build, Stage::Check]);
    let json = serde_json::to_value(&resumed).unwrap();
    assert_eq!(json["events"][0]["kind"], "blocked");
    assert_eq!(json["events"][1]["kind"], "resumed");
}

#[test]
fn gate_change_preserves_concurrent_pr_refresh_and_rejects_run_updates() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let url = "https://github.com/owner/repo/pull/1";
    store
        .update(
            &run.id,
            parse_update("define", "running", &["--pr", url]).unwrap(),
        )
        .unwrap();
    store
        .change_gate(&run.id, |next| {
            store
                .set_pr_status(
                    &run.id,
                    PrStatus {
                        url: url.into(),
                        state: crate::pr_state::PrState::Open,
                        reason: None,
                        checked_at: 42,
                    },
                )
                .unwrap();
            next.stage = Stage::Build;
            Ok(())
        })
        .unwrap();
    let current = store.get(&run.id).unwrap().unwrap();
    assert_eq!(current.stage, Stage::Build);
    assert_eq!(current.pr_status.unwrap().checked_at, 42);
    let error = store
        .change_gate(&run.id, |next| {
            store
                .update(
                    &run.id,
                    parse_update("build", "blocked", &["--blocking-reason", "waiting"]).unwrap(),
                )
                .unwrap();
            next.stage = Stage::Check;
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("changed while checking"));
    let current = store.get(&run.id).unwrap().unwrap();
    assert_eq!(current.stage, Stage::Build);
    assert_eq!(current.status, RunStatus::Blocked);
}

#[test]
fn current_ticket_accepts_local_references() {
    let dir = TestDir::new();
    let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
    let run = new_run(&store, "task");
    let updated = store
        .update(
            &run.id,
            parse_update(
                "build",
                "running",
                &[
                    "--ticket",
                    "/tmp/ticket.md Local ticket",
                    "--current-ticket",
                    "/tmp/ticket.md",
                ],
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.run.current_ticket.as_deref(),
        Some("/tmp/ticket.md")
    );
}
