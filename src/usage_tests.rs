//! Fixture-based tests: small log files written to a temporary folder, in the three agents' formats.
//! Nothing here reads a real user's logs or needs an agent installed.

use super::*;
use crate::run::{Run, Stage};
use serde_json::json;
use std::{ffi::OsString, fs, path::PathBuf, time::Duration};
use tempfile::TempDir;

/// 2026-11-05T10:00:00Z.
const T0: u64 = 1_793_872_800_000;
const RUN_A: &str = "1793872800000000001";
const RUN_B: &str = "1793872800000000002";

#[test]
fn cloned_store_collections_serialize_while_updates_remain_responsive() {
    use crate::run::{NewRun, RunStatus, RunStore, UpdateRun};
    use std::sync::mpsc;

    let world = World::new();
    let store = RunStore::open(&world.dir.path().join("runs.json")).unwrap();
    let run = store
        .create(NewRun {
            title: "task".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        })
        .unwrap();
    let settings = UsageSettings {
        enabled: false,
        roots: world.roots(),
        budget: Duration::from_secs(5),
        settle: Vec::new(),
    };
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first_store = store.clone();
    let first_settings = settings.clone();
    let id = run.id.clone();
    let first = std::thread::spawn(move || {
        first_store.usage_collection().collect_paused(
            &first_store,
            &id,
            &first_settings,
            None,
            entered_tx,
            release_rx,
        )
    });
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first collection entered");

    let (second_entered_tx, second_entered_rx) = mpsc::channel();
    let (second_release_tx, second_release_rx) = mpsc::channel();
    let (waiting_tx, waiting_rx) = mpsc::channel();
    let second_store = store.clone();
    let id = run.id.clone();
    let second_settings = settings.clone();
    let second = std::thread::spawn(move || {
        second_store.usage_collection().collect_paused(
            &second_store,
            &id,
            &second_settings,
            Some(waiting_tx),
            second_entered_tx,
            second_release_rx,
        )
    });
    waiting_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second collection waits for the first");
    let (updated_tx, updated_rx) = mpsc::channel();
    let update_store = store.clone();
    let id = run.id.clone();
    let update = std::thread::spawn(move || {
        let result = update_store.update(&id, UpdateRun::new(Stage::Build, RunStatus::Running));
        updated_tx.send(result).unwrap();
    });
    let updated = updated_rx.recv_timeout(Duration::from_secs(5));
    release_tx.send(()).unwrap();
    first.join().unwrap().unwrap();
    second_entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second collection entered after first");
    second_release_tx.send(()).unwrap();
    second.join().unwrap().unwrap();
    update.join().unwrap();
    assert!(updated
        .expect("update should complete while collection is held")
        .unwrap()
        .is_some());
    assert!(store.delete(&run.id).unwrap());
    collect_and_store(&store, &run.id, &settings).unwrap();
    assert!(store.get(&run.id).unwrap().is_none());
}

fn iso(offset_seconds: u64) -> String {
    iso_timestamp(T0 + offset_seconds * 1000)
}

fn run(
    id: &str,
    marks: &[(u64, &str, Option<u32>, Option<&str>)],
    finished: Option<u64>,
    sessions: &[&str],
) -> Run {
    let timeline: Vec<_> = marks
        .iter()
        .map(|(at, stage, rework, ticket)| json!({"at": T0 + at * 1000, "stage": stage, "rework": rework, "ticket": ticket}))
        .collect();
    serde_json::from_value(json!({
        "id": id, "title": "t", "project": "/p", "stage": "build", "status": "running",
        "timeline": timeline, "finished_at": finished.map(|s| T0 + s * 1000), "sessions": sessions,
        "tickets": [{"reference": "t1", "title": "One", "status": "done"}, {"reference": "t2", "title": "Two", "status": "pending"}]
    }))
    .unwrap()
}

/// A run that goes define, build on ticket t1, build on ticket t2, build with no ticket, check.
fn full_run(id: &str) -> Run {
    run(
        id,
        &[
            (0, "define", None, None),
            (100, "build", None, Some("t1")),
            (200, "build", None, Some("t2")),
            (300, "build", None, None),
            (400, "check", None, None),
        ],
        Some(500),
        &[],
    )
}

struct World {
    dir: TempDir,
}

impl World {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn roots(&self) -> Roots {
        let home = self.dir.path();
        Roots {
            claude: vec![home.join("claude/projects")],
            codex: home.join("codex/sessions"),
            copilot: home.join("copilot/session-state"),
        }
    }
    fn write(&self, relative: &str, lines: &[String]) {
        let path = self.dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }
    fn collect(&self, target: &Run, runs: &[Run]) -> Usage {
        collect(
            target,
            runs,
            &self.roots(),
            T0 + 10_000_000,
            Instant::now() + Duration::from_secs(30),
        )
    }
}

// --- fixture lines ----------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)] // a fixture line: every field is its own argument
fn claude_line(
    session: &str,
    at: u64,
    message: &str,
    request: &str,
    model: &str,
    usage: (u64, u64, u64, u64),
    thinking: u64,
    sidechain: bool,
) -> String {
    json!({
        "type": "assistant", "timestamp": iso(at), "sessionId": session, "requestId": request, "isSidechain": sidechain,
        "message": {"id": message, "model": model, "role": "assistant", "content": [{"type": "text", "text": "SECRET conversation text"}],
            "usage": {"input_tokens": usage.0, "cache_read_input_tokens": usage.1, "cache_creation_input_tokens": usage.2,
                      "output_tokens": usage.3, "output_tokens_details": {"thinking_tokens": thinking}}}
    })
    .to_string()
}

fn mention(run_id: &str) -> String {
    json!({"type": "user", "message": {"content": format!("factory run update {run_id} build running --activity SECRET prompt")}}).to_string()
}

/// A Claude Code response line that also records the reasoning effort of its turn.
fn with_effort(line: String, effort: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&line).unwrap();
    value["effort"] = json!(effort);
    value.to_string()
}

fn codex_context_with_effort(model: &str, effort: &str) -> String {
    json!({"type": "turn_context", "timestamp": iso(0), "payload": {"model": model, "effort": effort, "turn_id": "turn-1"}}).to_string()
}

fn codex_meta_line(id: &str, root: Option<&str>) -> String {
    let mut payload = json!({"id": id, "source": "cli"});
    if let Some(root) = root {
        payload["session_id"] = json!(root);
        payload["source"] = json!({"subagent": {"name": "reviewer"}});
    }
    json!({"type": "session_meta", "timestamp": iso(0), "payload": payload}).to_string()
}

fn codex_context(model: &str) -> String {
    json!({"type": "turn_context", "timestamp": iso(0), "payload": {"model": model, "turn_id": "turn-1"}}).to_string()
}

fn codex_usage(
    at: u64,
    response: &str,
    input: u64,
    cached: u64,
    written: u64,
    output: u64,
    reasoning: u64,
) -> String {
    json!({"type": "token_usage_record", "timestamp": iso(at), "payload": {"response_id": response, "turn_id": "turn-1",
        "usage": {"input_tokens": input, "cached_input_tokens": cached, "cache_write_input_tokens": written,
                  "output_tokens": output, "reasoning_output_tokens": reasoning, "total_tokens": input + output}}})
    .to_string()
}

#[allow(clippy::too_many_arguments)] // a fixture line: every field is its own argument
fn copilot_usage(
    at: u64,
    call: &str,
    model: &str,
    input: u64,
    read: u64,
    write: u64,
    output: u64,
    sub: bool,
) -> String {
    json!({"type": "session.usage_record", "timestamp": iso(at), "data": {"usage": {"model": model, "apiCallId": call,
        "inputTokens": input, "cacheReadTokens": read, "cacheWriteTokens": write, "outputTokens": output,
        "reasoningTokens": 3, "initiator": if sub {"sub-agent"} else {"user"}}}})
    .to_string()
}

fn rows_of(usage: &Usage, stage: Stage) -> Vec<&UsageRow> {
    usage.rows.iter().filter(|row| row.stage == stage).collect()
}

// --- time and numbers -------------------------------------------------------------------------------

#[test]
fn timestamps_round_trip_and_reject_anything_else() {
    assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_timestamp("2026-11-05T10:00:00Z"), Some(T0));
    assert_eq!(
        parse_timestamp("2026-11-05T21:17:49.123Z"),
        Some(1_793_913_469_123)
    );
    assert_eq!(
        parse_timestamp("2024-02-29T23:59:59Z"),
        Some(1_709_251_199_000)
    );
    assert_eq!(parse_timestamp("2026-11-05T11:00:00+01:00"), Some(T0));
    assert_eq!(parse_timestamp("2026-11-05T09:00:00-01:00"), Some(T0));
    assert_eq!(parse_timestamp("2026-11-05T10:00:00.5Z"), Some(T0 + 500));
    for bad in [
        "",
        "yesterday",
        "2026-11-05",
        "2026-13-05T10:00:00Z",
        "2026-11-05T25:00:00Z",
        "2026-11-05T10:00:00",
        "2026-11-05T10:00:00.Z",
        "2026-11-05T10:00:00+0100",
    ] {
        assert_eq!(parse_timestamp(bad), None, "{bad}");
    }
    assert_eq!(format_timestamp(T0 + 61_000), "2026-11-05 10:01 UTC");
}

#[test]
fn fresh_tokens_are_input_plus_cache_writes_plus_output_and_unknown_is_not_zero() {
    let tokens = Tokens {
        input: Some(10),
        cache_read: Some(1000),
        cache_write: Some(20),
        output: Some(5),
        reasoning: Some(2),
    };
    assert_eq!(
        tokens.fresh(),
        Some(35),
        "cache reads and reasoning are not added"
    );
    assert_eq!(
        Tokens {
            cache_read: Some(5),
            ..Tokens::default()
        }
        .fresh(),
        None,
        "nothing about input or output is unknown, not 0"
    );
    let mut sum = Tokens::default();
    sum.add(&Tokens {
        input: Some(1),
        ..Tokens::default()
    });
    sum.add(&Tokens {
        input: Some(2),
        output: Some(4),
        ..Tokens::default()
    });
    assert_eq!(
        (sum.input, sum.cache_read, sum.output),
        (Some(3), None, Some(4))
    );
}

// --- the three formats ------------------------------------------------------------------------------

#[test]
fn claude_usage_is_normalised_deduplicated_and_subagents_are_counted_once() {
    let world = World::new();
    let target = full_run(RUN_A);
    // The same response is written twice (once per content block); the parent also lists the subagent's
    // call, and the subagent's own transcript holds it again.
    world.write(
        "claude/projects/-work/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line(
                "S1",
                10,
                "m1",
                "r1",
                "claude-opus-test",
                (10, 100, 20, 5),
                2,
                false,
            ),
            claude_line(
                "S1",
                10,
                "m1",
                "r1",
                "claude-opus-test",
                (10, 100, 20, 5),
                2,
                false,
            ),
            claude_line(
                "S1",
                20,
                "m2",
                "r2",
                "claude-sonnet-test",
                (1, 0, 0, 7),
                0,
                false,
            ),
            claude_line(
                "S1",
                30,
                "m3",
                "r3",
                "claude-opus-test",
                (3, 50, 0, 4),
                0,
                true,
            ),
        ],
    );
    world.write(
        "claude/projects/-work/S1/subagents/agent-a.jsonl",
        &[
            claude_line(
                "S1",
                30,
                "m3",
                "r3",
                "claude-opus-test",
                (3, 50, 0, 4),
                0,
                true,
            ),
            claude_line(
                "S1",
                31,
                "m4",
                "r4",
                "claude-opus-test",
                (2, 10, 0, 6),
                0,
                true,
            ),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(
        usage.status,
        UsageStatus::Partial,
        "build and check have no data"
    );
    assert_eq!(usage.sessions, ["S1"]);
    let define = rows_of(&usage, Stage::Define);
    assert_eq!(define.len(), 2, "one row per model");
    let opus = define
        .iter()
        .find(|r| r.model == "claude-opus-test")
        .unwrap();
    // 10 + 3 + 2 input, 100 + 50 + 10 cache read, 20 cache write, 5 + 4 + 6 output; each response once.
    assert_eq!(
        opus.tokens,
        Tokens {
            input: Some(15),
            cache_read: Some(160),
            cache_write: Some(20),
            output: Some(15),
            reasoning: Some(2)
        }
    );
    assert_eq!(opus.tokens.fresh(), Some(15 + 20 + 15));
    // The subagent share is part of the total, not added to it.
    assert_eq!(opus.subagents.input, Some(5));
    assert_eq!(opus.subagents.output, Some(10));
    let sonnet = define
        .iter()
        .find(|r| r.model == "claude-sonnet-test")
        .unwrap();
    assert_eq!(sonnet.tokens.cache_read, Some(0));
    assert_eq!(usage.no_data, [Stage::Build, Stage::Check]);
}

#[test]
fn sessions_are_read_from_every_claude_root() {
    let world = World::new();
    let target = full_run(RUN_A);
    for (root, session, request, at) in
        [("claude", "S1", "r1", 10), ("claude-work", "S2", "r2", 20)]
    {
        world.write(
            &format!("{root}/projects/-work/{session}.jsonl"),
            &[
                mention(RUN_A),
                claude_line(
                    session,
                    at,
                    &format!("m-{session}"),
                    request,
                    "claude-opus-test",
                    (1, 0, 0, 2),
                    0,
                    false,
                ),
            ],
        );
    }
    let mut roots = world.roots();
    roots
        .claude
        .push(world.dir.path().join("claude-work/projects"));
    let usage = collect(
        &target,
        std::slice::from_ref(&target),
        &roots,
        T0 + 10_000_000,
        Instant::now() + Duration::from_secs(30),
    );
    assert_eq!(usage.sessions, ["S1", "S2"]);
}

#[test]
fn claude_roots_are_the_resolved_dir_plus_the_listed_ones_without_duplicates() {
    use crate::install::test_abs;
    let home = test_abs("home/user");
    let env = test_abs("env");
    let work = test_abs("work");
    let list = std::env::join_paths([
        work.clone(),
        PathBuf::new(),
        PathBuf::from("rel"),
        PathBuf::from("~/other"),
        work.clone(),
        env.clone(),
    ])
    .unwrap();
    let var = |name: &str| match name {
        "CLAUDE_CONFIG_DIR" => Some(env.clone().into_os_string()),
        "FACTORY_CLAUDE_CONFIG_DIRS" => Some(list.clone()),
        _ => None,
    };
    let roots = Roots::from_home(&home, &var).unwrap();
    assert_eq!(
        roots.claude,
        [
            env.join("projects"),
            work.join("projects"),
            home.join("other/projects"),
        ]
    );
    let roots = Roots::from_home(&home, &|_| None).unwrap();
    assert_eq!(roots.claude, [home.join(".claude/projects")]);
    let empty = |name: &str| (name == "FACTORY_CLAUDE_CONFIG_DIRS").then(OsString::new);
    assert_eq!(Roots::from_home(&home, &empty).unwrap().claude.len(), 1);
}

#[test]
fn claude_efforts_are_kept_per_model_and_a_subagent_keeps_its_own() {
    let world = World::new();
    let target = full_run(RUN_A);
    let line = |at: u64, id: &str, model: &str, sidechain: bool| {
        claude_line("S1", at, id, id, model, (10, 0, 0, 5), 0, sidechain)
    };
    world.write(
        "claude/projects/-work/S1.jsonl",
        &[
            mention(RUN_A),
            with_effort(line(10, "m1", "claude-sonnet-test", false), "medium"),
            with_effort(line(11, "m2", "claude-sonnet-test", false), "high"),
            // No effort in this response: nothing is guessed.
            line(12, "m3", "claude-haiku-test", false),
            // An effort that is not plain text is dropped, not shown.
            with_effort(line(13, "m4", "claude-fable-test", false), "hi\u{1b}[2Jgh"),
        ],
    );
    world.write(
        "claude/projects/-work/S1/subagents/agent-a.jsonl",
        &[with_effort(
            line(14, "m5", "claude-opus-test", true),
            "xhigh",
        )],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    let rows = rows_of(&usage, Stage::Define);
    let efforts = |model: &str| {
        rows.iter()
            .find(|r| r.model == model)
            .unwrap()
            .efforts
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(efforts("claude-sonnet-test"), ["high", "medium"]);
    assert_eq!(efforts("claude-opus-test"), ["xhigh"]);
    assert!(efforts("claude-haiku-test").is_empty());
    assert!(efforts("claude-fable-test").is_empty());
}

#[test]
fn codex_efforts_follow_the_turn_and_a_subagent_rollout_has_its_own() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-abc.jsonl",
        &[
            codex_meta_line("root-1", None),
            codex_context_with_effort("gpt-main", "medium"),
            mention(RUN_A),
            codex_usage(10, "resp-1", 100, 0, 0, 10, 0),
        ],
    );
    world.write(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-30-def.jsonl",
        &[
            codex_meta_line("sub-1", Some("root-1")),
            codex_context_with_effort("gpt-sub", "high"),
            codex_usage(30, "resp-3", 90, 0, 0, 9, 0),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    let rows = rows_of(&usage, Stage::Define);
    let efforts = |model: &str| {
        rows.iter()
            .find(|r| r.model == model)
            .unwrap()
            .efforts
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(efforts("gpt-main"), ["medium"]);
    assert_eq!(efforts("gpt-sub"), ["high"]);
}

fn codex_turn_context(turn: &str, model: &str, effort: Option<&str>) -> String {
    let mut payload = json!({"model": model, "turn_id": turn});
    if let Some(effort) = effort {
        payload["effort"] = json!(effort);
    }
    json!({"type": "turn_context", "timestamp": iso(0), "payload": payload}).to_string()
}

fn codex_turn_usage(at: u64, turn: &str, response: &str) -> String {
    json!({"type": "token_usage_record", "timestamp": iso(at), "payload": {"response_id": response, "turn_id": turn,
        "usage": {"input_tokens": 10, "output_tokens": 5}}})
    .to_string()
}

#[test]
fn codex_effort_follows_each_turn_and_an_unknown_turn_takes_the_last_one() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-abc.jsonl",
        &[
            codex_meta_line("root-1", None),
            mention(RUN_A),
            codex_turn_context("turn-1", "gpt-a", Some("medium")),
            codex_turn_context("turn-2", "gpt-b", Some("high")),
            // Each usage record belongs to its own turn, whatever came last.
            codex_turn_usage(10, "turn-1", "resp-1"),
            // This turn has no context of its own: the last one (gpt-b, high) applies.
            codex_turn_usage(20, "turn-9", "resp-2"),
            // A later turn that records no effort resets what is carried over.
            codex_turn_context("turn-3", "gpt-c", None),
            codex_turn_usage(30, "turn-3", "resp-3"),
            codex_turn_usage(40, "turn-8", "resp-4"),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    let rows = rows_of(&usage, Stage::Define);
    let efforts = |model: &str| {
        rows.iter()
            .find(|r| r.model == model)
            .unwrap()
            .efforts
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(efforts("gpt-a"), ["medium"]);
    assert_eq!(efforts("gpt-b"), ["high"]);
    assert!(
        efforts("gpt-c").is_empty(),
        "no effort is carried over from an older turn"
    );
}

#[test]
fn a_log_cannot_make_a_row_hold_more_than_a_handful_of_efforts() {
    let world = World::new();
    let target = full_run(RUN_A);
    let mut lines = vec![mention(RUN_A)];
    for n in 0..20 {
        lines.push(with_effort(
            claude_line(
                "S1",
                10 + n,
                &format!("m{n}"),
                &format!("r{n}"),
                "claude-sonnet-test",
                (1, 0, 0, 1),
                0,
                false,
            ),
            &format!("e{n:02}"),
        ));
    }
    world.write("claude/projects/-work/S1.jsonl", &lines);
    let usage = world.collect(&target, std::slice::from_ref(&target));
    let row = rows_of(&usage, Stage::Define)[0];
    assert_eq!(row.efforts.len(), MAX_EFFORTS_PER_ROW);
    assert!(row.efforts.contains("e00") && !row.efforts.contains("e19"));
    // The tokens are still all counted.
    assert_eq!(row.tokens.output, Some(20));
}

#[test]
fn copilot_rows_have_no_effort_and_old_usage_records_load_without_the_field() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "copilot/session-state/c1/events.jsonl",
        &[
            mention(RUN_A),
            copilot_usage(10, "call-1", "gpt-copilot", 100, 10, 0, 5, false),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert!(rows_of(&usage, Stage::Define)
        .iter()
        .all(|r| r.efforts.is_empty()));
    // The stored form leaves the key out when there is nothing to say, and reads a record without it.
    let json = serde_json::to_value(&usage.rows[0]).unwrap();
    assert!(json.get("efforts").is_none(), "{json}");
    let old: UsageRow =
        serde_json::from_value(json!({"stage": "build", "model": "m", "input": 1})).unwrap();
    assert!(old.efforts.is_empty());
    let new: UsageRow =
        serde_json::from_value(json!({"stage": "build", "model": "m", "efforts": ["high"]}))
            .unwrap();
    assert_eq!(new.efforts.len(), 1);
    assert!(new.efforts.contains("high"));
}

#[test]
fn codex_cached_input_moves_to_cache_read_and_reasoning_stays_inside_output() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-abc.jsonl",
        &[
            codex_meta_line("root-1", None),
            codex_context("gpt-test"),
            mention(RUN_A),
            codex_usage(10, "resp-1", 1000, 600, 100, 50, 20),
            codex_usage(10, "resp-1", 1000, 600, 100, 50, 20),
            codex_usage(20, "resp-2", 200, 0, 0, 10, 0),
        ],
    );
    // A subagent's rollout names the run only through its root session.
    world.write(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-30-def.jsonl",
        &[
            codex_meta_line("sub-1", Some("root-1")),
            codex_context("gpt-sub"),
            codex_usage(30, "resp-3", 90, 40, 0, 9, 0),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    let rows = rows_of(&usage, Stage::Define);
    let main = rows.iter().find(|r| r.model == "gpt-test").unwrap();
    // 1000 input includes 600 cached and 100 written: 300 fresh input.
    assert_eq!(main.tokens.input, Some(300 + 200));
    assert_eq!(main.tokens.cache_read, Some(600));
    assert_eq!(main.tokens.cache_write, Some(100));
    assert_eq!(main.tokens.output, Some(60));
    assert_eq!(
        main.tokens.reasoning,
        Some(20),
        "reasoning is reported inside output"
    );
    assert_eq!(main.tokens.fresh(), Some(500 + 100 + 60));
    let sub = rows.iter().find(|r| r.model == "gpt-sub").unwrap();
    assert_eq!(sub.tokens.input, Some(50));
    assert_eq!(
        sub.subagents.output,
        Some(9),
        "a subagent rollout is counted, flagged as a subagent"
    );
    assert_eq!(usage.sessions, ["root-1"]);
}

#[test]
fn copilot_cache_reads_and_writes_are_taken_out_of_input() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "copilot/session-state/cop-session-1/events.jsonl",
        &[
            mention(RUN_A),
            copilot_usage(10, "call-1", "gpt-copilot", 1000, 700, 100, 40, false),
            copilot_usage(10, "call-1", "gpt-copilot", 1000, 700, 100, 40, false),
            copilot_usage(20, "call-2", "gpt-copilot", 50, 0, 0, 5, true),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    let row = &rows_of(&usage, Stage::Define)[0];
    assert_eq!(row.tokens.input, Some(200 + 50));
    assert_eq!(row.tokens.cache_read, Some(700));
    assert_eq!(row.tokens.cache_write, Some(100));
    assert_eq!(row.tokens.output, Some(45));
    assert_eq!(row.subagents.output, Some(5));
}

// --- attribution ------------------------------------------------------------------------------------

#[test]
fn usage_is_split_by_stage_ticket_rework_and_work_without_a_ticket() {
    let world = World::new();
    let target = run(
        RUN_A,
        &[
            (0, "define", None, None),
            (100, "build", None, Some("t1")),
            (200, "build", None, Some("t2")),
            (300, "build", None, None),
            (400, "check", None, None),
            (450, "build", Some(1), None),
            (480, "build", Some(1), Some("t2")),
            (490, "check", Some(1), None),
        ],
        None,
        &[],
    );
    let model = "claude-opus-test";
    let lines: Vec<String> = [
        (10, "d"),
        (110, "t1"),
        (210, "t2"),
        (310, "wrap"),
        (410, "chk"),
        (460, "rw"),
        (485, "rw2"),
        (495, "chk2"),
    ]
    .iter()
    .map(|(at, id)| claude_line("S1", *at, id, id, model, (1, 0, 0, 1), 0, false))
    .collect();
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[vec![mention(RUN_A)], lines].concat(),
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.status, UsageStatus::Tracked);
    let key = |row: &UsageRow| (row.stage, row.ticket.clone(), row.rework);
    let got: Vec<_> = usage.rows.iter().map(key).collect();
    assert_eq!(
        got,
        [
            (Stage::Define, None, false),
            (Stage::Build, Some("t1".to_string()), false),
            (Stage::Build, Some("t2".to_string()), false),
            (Stage::Build, None, true),
            (Stage::Build, None, false),
            (Stage::Check, None, false),
        ],
        "tickets in the run's order, then the rework, then work with no ticket"
    );
    // Both rework entries (even with a ticket in progress) are in the rework bucket; both Check entries are Check.
    let rework = usage.rows.iter().find(|r| r.rework).unwrap();
    assert_eq!(rework.tokens.output, Some(2));
    assert_eq!(
        usage
            .rows
            .iter()
            .find(|r| r.stage == Stage::Check)
            .unwrap()
            .tokens
            .output,
        Some(2)
    );
    assert!(usage.no_data.is_empty());
}

#[test]
fn entries_outside_the_runs_window_are_ignored() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line(
                "S1",
                u64::MAX / 4_000,
                "late",
                "late",
                "m-1",
                (9, 0, 0, 9),
                0,
                false,
            ),
            claude_line("S1", 10, "in", "in", "m-1", (1, 0, 0, 1), 0, false),
            claude_line("S1", 600, "after", "after", "m-1", (9, 0, 0, 9), 0, false),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.rows.len(), 1);
    assert_eq!(usage.rows[0].tokens.input, Some(1));
}

#[test]
fn concurrent_runs_in_separate_sessions_never_mix() {
    let world = World::new();
    let a = full_run(RUN_A);
    let b = full_run(RUN_B);
    world.write(
        "claude/projects/-same-folder/SA.jsonl",
        &[
            mention(RUN_A),
            claude_line("SA", 10, "a1", "a1", "m-1", (100, 0, 0, 10), 0, false),
        ],
    );
    world.write(
        "claude/projects/-same-folder/SB.jsonl",
        &[
            mention(RUN_B),
            claude_line("SB", 10, "b1", "b1", "m-1", (7, 0, 0, 3), 0, false),
        ],
    );
    let runs = [a.clone(), b.clone()];
    let for_a = world.collect(&a, &runs);
    let for_b = world.collect(&b, &runs);
    assert_eq!(for_a.rows[0].tokens.input, Some(100));
    assert_eq!(for_b.rows[0].tokens.input, Some(7));
    assert_eq!(for_a.sessions, ["SA"]);
    assert_eq!(for_b.sessions, ["SB"]);
}

#[test]
fn a_session_shared_by_runs_working_at_the_same_time_is_not_divided_by_guessing() {
    let world = World::new();
    let a = full_run(RUN_A);
    let b = full_run(RUN_B);
    // One session names both runs, and both are in their window at the same moments.
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            mention(RUN_B),
            claude_line("S1", 10, "x1", "x1", "m-1", (40, 0, 0, 2), 0, false),
        ],
    );
    let runs = [a.clone(), b.clone()];
    for target in [&a, &b] {
        let usage = world.collect(target, &runs);
        assert!(
            usage.rows.is_empty() || usage.rows.iter().all(|r| r.tokens.is_empty()),
            "{usage:?}"
        );
        assert_eq!(usage.unattributed.fresh(), Some(42));
        assert!(
            usage
                .notes
                .iter()
                .any(|n| n.contains("shared with another run")),
            "{usage:?}"
        );
        assert_ne!(usage.status, UsageStatus::Tracked);
    }
}

#[test]
fn a_session_that_moves_from_one_run_to_the_next_is_cut_by_each_runs_window() {
    let world = World::new();
    let a = run(RUN_A, &[(0, "define", None, None)], Some(100), &[]);
    let b = run(RUN_B, &[(200, "define", None, None)], None, &[]);
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            mention(RUN_B),
            claude_line("S1", 50, "a", "a", "m-1", (10, 0, 0, 1), 0, false),
            claude_line("S1", 250, "b", "b", "m-1", (20, 0, 0, 2), 0, false),
        ],
    );
    let runs = [a.clone(), b.clone()];
    let for_a = world.collect(&a, &runs);
    let for_b = world.collect(&b, &runs);
    assert_eq!(for_a.status, UsageStatus::Tracked);
    assert_eq!(for_a.rows[0].tokens.input, Some(10));
    assert_eq!(for_b.rows[0].tokens.input, Some(20));
    assert!(for_a.unattributed.is_empty() && for_b.unattributed.is_empty());
}

#[test]
fn resumed_sessions_are_all_counted_and_a_repeated_response_only_once() {
    let world = World::new();
    let target = run(RUN_A, &[(0, "define", None, None)], None, &["S-first"]);
    // The first session was recorded for the run (it never names the id). The resumed session repeats an old
    // response and adds new ones.
    world.write(
        "claude/projects/-w/S-first.jsonl",
        &[claude_line(
            "S-first",
            10,
            "m1",
            "r1",
            "m-1",
            (5, 0, 0, 1),
            0,
            false,
        )],
    );
    world.write(
        "claude/projects/-w/S-second.jsonl",
        &[
            mention(RUN_A),
            claude_line("S-second", 10, "m1", "r1", "m-1", (5, 0, 0, 1), 0, false),
            claude_line("S-second", 20, "m2", "r2", "m-1", (7, 0, 0, 2), 0, false),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.rows[0].tokens.input, Some(12));
    assert_eq!(usage.rows[0].tokens.output, Some(3));
    assert_eq!(usage.sessions.len(), 2);
}

#[test]
fn collecting_again_gives_the_same_numbers() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line("S1", 10, "m", "r", "m-1", (3, 4, 5, 6), 0, false),
        ],
    );
    let first = world.collect(&target, std::slice::from_ref(&target));
    let second = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(first.rows, second.rows);
    assert_eq!(first.sessions, second.sessions);
}

// --- coverage ---------------------------------------------------------------------------------------

#[test]
fn missing_unreadable_or_unrecognised_logs_are_not_tracked_with_a_reason_never_zero() {
    let world = World::new();
    let target = full_run(RUN_A);
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.status, UsageStatus::NotTracked);
    assert!(usage
        .reason
        .as_deref()
        .unwrap()
        .contains("no Claude Code, Codex or Copilot session logs"));
    assert!(usage.rows.is_empty());

    // Logs exist but none names the run.
    world.write(
        "claude/projects/-w/S9.jsonl",
        &[claude_line(
            "S9",
            10,
            "m",
            "r",
            "m-1",
            (1, 0, 0, 1),
            0,
            false,
        )],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.status, UsageStatus::NotTracked);
    assert!(usage
        .reason
        .as_deref()
        .unwrap()
        .contains("no session log names this run"));

    // A session that names the run but is in a format nobody reads.
    world.write(
        "claude/projects/-w/S2.jsonl",
        &[
            mention(RUN_A),
            json!({"type": "assistant", "usage": {"tokens": "many"}}).to_string(),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.status, UsageStatus::NotTracked);
    assert!(
        usage
            .notes
            .iter()
            .any(|n| n.contains("no usage entries in a recognised format")),
        "{usage:?}"
    );
}

#[test]
fn malformed_lines_are_skipped_and_the_coverage_is_partial() {
    let world = World::new();
    let target = run(RUN_A, &[(0, "define", None, None)], None, &[]);
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            "{\"usage\": truncated".to_string(),
            claude_line("S1", 10, "m", "r", "m-1", (3, 0, 0, 1), 0, false),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.status, UsageStatus::Partial);
    assert_eq!(usage.rows[0].tokens.input, Some(3));
    assert!(
        usage
            .notes
            .iter()
            .any(|n| n.contains("1 malformed Claude Code log line(s) were skipped")),
        "{usage:?}"
    );
}

#[test]
fn a_field_the_log_does_not_report_stays_absent() {
    let world = World::new();
    let target = run(RUN_A, &[(0, "define", None, None)], None, &[]);
    let line = json!({"type": "assistant", "timestamp": iso(10), "requestId": "r", "message": {"id": "m", "model": "m-1",
        "usage": {"input_tokens": 4, "output_tokens": 2}}})
    .to_string();
    world.write("claude/projects/-w/S1.jsonl", &[mention(RUN_A), line]);
    let usage = world.collect(&target, std::slice::from_ref(&target));
    let tokens = usage.rows[0].tokens;
    assert_eq!((tokens.input, tokens.output), (Some(4), Some(2)));
    assert_eq!(
        (tokens.cache_read, tokens.cache_write, tokens.reasoning),
        (None, None, None)
    );
    let stored = serde_json::to_value(&usage).unwrap();
    assert!(
        stored["rows"][0].get("cache_read").is_none(),
        "absent, not 0: {stored}"
    );
}

#[test]
fn a_run_from_before_usage_tracking_is_not_tracked_and_a_late_timeline_is_partial() {
    let world = World::new();
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line("S1", 10, "m", "r", "m-1", (3, 0, 0, 1), 0, false),
        ],
    );
    let legacy: Run = serde_json::from_value(json!({"id": RUN_A, "title": "old", "project": "/p", "stage": "build", "status": "running"})).unwrap();
    let usage = world.collect(&legacy, std::slice::from_ref(&legacy));
    assert_eq!(usage.status, UsageStatus::NotTracked);
    assert!(usage
        .reason
        .as_deref()
        .unwrap()
        .contains("started before usage tracking"));

    // The timeline starts an hour after the run was created (its id is its creation time).
    let late = run(RUN_A, &[(3600, "define", None, None)], None, &[]);
    world.write(
        "claude/projects/-w/S2.jsonl",
        &[
            mention(RUN_A),
            claude_line("S2", 3700, "m2", "r2", "m-1", (3, 0, 0, 1), 0, false),
        ],
    );
    let usage = world.collect(&late, std::slice::from_ref(&late));
    assert_eq!(usage.status, UsageStatus::Partial);
    assert!(
        usage.notes.iter().any(|n| n.contains("usage before")),
        "{usage:?}"
    );
}

#[test]
fn a_tight_deadline_is_reported_instead_of_blocking() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line("S1", 10, "m", "r", "m-1", (3, 0, 0, 1), 0, false),
        ],
    );
    let usage = collect(
        &target,
        std::slice::from_ref(&target),
        &world.roots(),
        T0,
        Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now),
    );
    assert_eq!(usage.status, UsageStatus::NotTracked);
    assert!(
        usage.reason.as_deref().unwrap().contains("took too long"),
        "{usage:?}"
    );
}

// --- privacy ----------------------------------------------------------------------------------------

#[test]
fn nothing_but_counts_and_identifiers_is_kept() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line(
                "S1",
                10,
                "m",
                "r",
                "claude-opus-test",
                (3, 0, 0, 1),
                0,
                false,
            ),
        ],
    );
    world.write(
        "codex/sessions/2026/11/05/rollout-x.jsonl",
        &[
            codex_meta_line("c-1", None),
            mention(RUN_A),
            codex_context("gpt-test"),
            codex_usage(10, "x", 5, 0, 0, 1, 0),
        ],
    );
    let stored =
        serde_json::to_string(&world.collect(&target, std::slice::from_ref(&target))).unwrap();
    for text in ["SECRET", "prompt", "conversation", "factory run update"] {
        assert!(!stored.contains(text), "{text} leaked into {stored}");
    }
    // Only these keys can appear.
    let value: serde_json::Value = serde_json::from_str(&stored).unwrap();
    for row in value["rows"].as_array().unwrap() {
        for key in row.as_object().unwrap().keys() {
            assert!(
                [
                    "stage",
                    "model",
                    "ticket",
                    "rework",
                    "input",
                    "cache_read",
                    "cache_write",
                    "output",
                    "reasoning",
                    "subagents"
                ]
                .contains(&key.as_str()),
                "{key}"
            );
        }
    }
}

#[test]
fn a_model_name_that_is_not_a_plain_name_is_dropped() {
    let world = World::new();
    let target = run(RUN_A, &[(0, "define", None, None)], None, &[]);
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line(
                "S1",
                10,
                "m",
                "r",
                "<script>alert(1)</script>",
                (3, 0, 0, 1),
                0,
                false,
            ),
            claude_line("S1", 11, "m2", "r2", "ok-model", (1, 0, 0, 1), 0, false),
        ],
    );
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.rows.len(), 1);
    assert_eq!(usage.rows[0].model, "ok-model");
}

#[test]
fn only_a_run_command_naming_the_whole_run_id_counts_as_naming_the_run() {
    let ids: HashSet<&str> = [RUN_A].into_iter().collect();
    for yes in [
        format!("factory run update {RUN_A} build running"),
        format!("factory run update \"{RUN_A}\" build running"),
        format!("{{\"command\":\"factory run inspect {RUN_A} --json\"}}"),
        format!("factory run stats {RUN_A}"),
    ] {
        assert!(
            !crate::usage_logs::mentioned_runs(&yes, &ids).is_empty(),
            "{yes}"
        );
    }
    for no in [
        // The id as output or data, not as the target of a command: a session that listed runs names them all.
        format!("Run {RUN_A}: Task"),
        format!("run start printed {RUN_A}"),
        format!("factory run update {RUN_A}0"),
        format!("factory run update 0{RUN_A}"),
        "factory run update 1793872800000000009".to_string(),
        "factory run update 17938728".to_string(),
        format!("see {RUN_A}"),
    ] {
        assert!(
            crate::usage_logs::mentioned_runs(&no, &ids).is_empty(),
            "{no}"
        );
    }
}
