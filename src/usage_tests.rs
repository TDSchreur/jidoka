//! Fixture-based tests: small log files written to a temporary folder, in the three agents' formats.
//! Nothing here reads a real user's logs or needs an agent installed.

use super::*;
use crate::run::{Run, Stage};
use serde_json::json;
use std::{fs, time::Duration};
use tempfile::TempDir;

/// 2026-11-05T10:00:00Z.
const T0: u64 = 1_793_872_800_000;
const RUN_A: &str = "1793872800000000001";
const RUN_B: &str = "1793872800000000002";

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
        "id": id, "title": "t", "gates": [], "project": "/p", "stage": "build", "status": "running",
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

#[expect(
    clippy::too_many_arguments,
    reason = "a fixture line: every field is its own argument"
)]
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
    json!({"type": "user", "message": {"content": format!("jidoka run update {run_id} build running --activity SECRET prompt")}}).to_string()
}

/// A Claude Code response line that also records the reasoning effort of its turn.
fn with_effort(line: String, effort: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&line).unwrap();
    value["effort"] = json!(effort);
    value.to_string()
}

/// A `session_meta` line as Codex 0.167 writes it: a subagent rollout names its root session and the
/// `thread_spawn` that created it.
fn codex_meta_line(id: &str, root: Option<&str>) -> String {
    let mut payload = json!({"id": id, "session_id": id, "source": "cli", "thread_source": "user"});
    if let Some(root) = root {
        payload["session_id"] = json!(root);
        payload["thread_source"] = json!("subagent");
        payload["source"] = json!({"subagent": {"thread_spawn": {"parent_thread_id": root, "depth": 1,
            "agent_path": "/root/reviewer", "agent_nickname": "Rev", "agent_role": "reviewer"}}});
    }
    json!({"type": "session_meta", "timestamp": iso(0), "payload": payload}).to_string()
}

/// A command the agent ran, as Codex writes an executed command.
fn codex_command_line(script: &str, output: &str) -> String {
    json!({"type": "event_msg", "timestamp": iso(0), "payload": {"type": "item_completed", "item": {
        "type": "CommandExecution", "command": ["/usr/bin/bash", "-lc", script],
        "parsed_cmd": [{"type": "unknown", "cmd": script}], "aggregated_output": output}}})
    .to_string()
}

fn codex_command(run_id: &str) -> String {
    codex_command_line(&format!("jidoka run update {run_id} build running"), "")
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

#[expect(
    clippy::too_many_arguments,
    reason = "a fixture line: every field is its own argument"
)]
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
        "reasoningTokens": 3, "reasoningEffort": "medium", "cacheDetailsReported": true,
        "initiator": if sub {"sub-agent"} else {"agent"}}}})
    .to_string()
}

fn rows_of(usage: &Usage, stage: Stage) -> Vec<&UsageRow> {
    usage.rows.iter().filter(|row| row.stage == stage).collect()
}

// --- time and numbers -------------------------------------------------------------------------------

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
fn codex_cached_input_moves_to_cache_read_and_reasoning_stays_inside_output() {
    let world = World::new();
    let target = full_run(RUN_A);
    world.write(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-abc.jsonl",
        &[
            codex_meta_line("root-1", None),
            codex_context("gpt-test"),
            codex_command(RUN_A),
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

// --- coverage ---------------------------------------------------------------------------------------

#[test]
fn missing_unreadable_or_unrecognised_logs_are_not_tracked_with_a_reason_never_zero() {
    let world = World::new();
    let target = full_run(RUN_A);
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(usage.status, UsageStatus::NotTracked);
    assert!(
        usage
            .reason
            .as_deref()
            .unwrap()
            .contains("no Claude Code, Codex or Copilot session logs")
    );
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
    assert!(
        usage
            .reason
            .as_deref()
            .unwrap()
            .contains("no session log names this run")
    );

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
fn a_run_from_before_usage_tracking_is_not_tracked_and_a_late_timeline_is_partial() {
    let world = World::new();
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line("S1", 10, "m", "r", "m-1", (3, 0, 0, 1), 0, false),
        ],
    );
    let legacy: Run = serde_json::from_value(json!({"id": RUN_A, "title": "old", "gates": [], "project": "/p", "stage": "build", "status": "running"})).unwrap();
    let usage = world.collect(&legacy, std::slice::from_ref(&legacy));
    assert_eq!(usage.status, UsageStatus::NotTracked);
    assert!(
        usage
            .reason
            .as_deref()
            .unwrap()
            .contains("started before usage tracking")
    );

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

// --- privacy ----------------------------------------------------------------------------------------

// --- pauses and idle gaps ---------------------------------------------------------------------------

fn usage_limit_line(at: u64) -> String {
    json!({"type": "assistant", "isApiErrorMessage": true, "error": "rate_limit", "timestamp": iso(at),
        "message": {"model": "<synthetic>", "content": [{"type": "text", "text": "SECRET limit text"}],
            "usage": {"input_tokens": 0, "output_tokens": 0}}})
    .to_string()
}

fn activity_line(kind: &str, at: u64) -> String {
    json!({"type": kind, "timestamp": iso(at), "message": {"content": "SECRET text"}}).to_string()
}

/// A run from second 0 whose Claude session `S1` names it, with the given extra log lines.
fn pause_world(lines: Vec<String>) -> (World, Run) {
    let world = World::new();
    let target = run(RUN_A, &[(0, "build", None, None)], None, &[]);
    let mut all = vec![mention(RUN_A)];
    all.extend(lines);
    world.write("claude/projects/-w/S1.jsonl", &all);
    (world, target)
}

fn at(seconds: u64) -> u64 {
    T0 + seconds * 1000
}

#[test]
fn a_usage_limit_with_no_later_activity_is_an_unresumed_pause_and_is_not_a_model_row() {
    let (world, target) = pause_world(vec![activity_line("assistant", 10), usage_limit_line(20)]);
    let usage = world.collect(&target, std::slice::from_ref(&target));
    assert_eq!(
        usage.pauses,
        vec![Pause {
            kind: PauseKind::UsageLimit,
            at: at(20),
            resumed_at: None
        }]
    );
    assert!(usage.idle.is_empty());
    // The synthetic limit message is neither a row nor a note about a skipped line.
    assert!(usage.rows.is_empty());
    assert!(
        !usage
            .notes
            .iter()
            .any(|note| note.contains("no usable model"))
    );
}

// --- blocking and resuming through the store --------------------------------------------------------

mod auto_block {
    use super::*;
    use crate::run::{FieldUpdate, RunEventKind, RunStatus, RunStore, UpdateRun};

    struct Fixture {
        world: World,
        store: RunStore,
        settings: UsageSettings,
        id: String,
        /// When the run's timeline began, in milliseconds.
        began: u64,
        lines: Vec<String>,
    }

    impl Fixture {
        fn new() -> Self {
            let world = World::new();
            // A run that began six hours ago, so the log moments after it are in the past.
            let began = crate::run::updated_now().unwrap() - 6 * 3_600_000;
            let id = format!("{}", u128::from(began) * 1_000_000);
            let path = world.dir.path().join("runs.json");
            fs::write(
                &path,
                json!([{"id": id, "title": "task", "gates": [], "project": "/work", "stage": "build", "status": "running",
                    "updated_at": began, "timeline": [{"at": began, "stage": "build"}]}])
                .to_string(),
            )
            .unwrap();
            let store = RunStore::open(&path).unwrap();
            let settings = UsageSettings {
                enabled: true,
                roots: world.roots(),
                budget: Duration::from_secs(10),
                settle: Vec::new(),
            };
            Self {
                world,
                store,
                settings,
                lines: vec![mention(&id)],
                id,
                began,
            }
        }

        /// Appends a log line `ms` after the run began and collects.
        fn log(&mut self, make: impl Fn(&str) -> String, ms: u64) -> Run {
            let ts = iso_timestamp(self.began + ms);
            self.lines.push(make(&ts));
            self.world.write("claude/projects/-w/S1.jsonl", &self.lines);
            collect_and_store(&self.store, &self.id, &self.settings).unwrap();
            self.run()
        }

        fn limit(&mut self, ms: u64) -> Run {
            self.log(
                |ts| json!({"type": "assistant", "isApiErrorMessage": true, "error": "rate_limit", "timestamp": ts,
                    "message": {"model": "<synthetic>", "usage": {"input_tokens": 0, "output_tokens": 0}}}).to_string(),
                ms,
            )
        }

        fn activity(&mut self, ms: u64) -> Run {
            self.log(
                |ts| {
                    json!({"type": "assistant", "timestamp": ts, "message": {"content": "x"}})
                        .to_string()
                },
                ms,
            )
        }

        fn run(&self) -> Run {
            self.store.get(&self.id).unwrap().unwrap()
        }

        fn agent_update(&self, status: RunStatus, reason: Option<&str>) {
            let mut update = UpdateRun::new(status);
            if status == RunStatus::Done {
                update
                    .summaries
                    .insert(Stage::Finish, Some("Shipped".into()));
            }
            if let Some(reason) = reason {
                update.blocking_reason = FieldUpdate::Set(reason.into());
            }
            self.store.update(&self.id, update).unwrap();
        }
    }

    #[test]
    fn a_usage_limit_blocks_the_run_and_later_activity_resumes_it() {
        let mut f = Fixture::new();
        let blocked = f.limit(1_000);
        assert_eq!(blocked.status, RunStatus::Blocked);
        assert_eq!(
            blocked.blocking_reason.as_deref(),
            Some("Usage limit reached")
        );
        let event = blocked.events.last().unwrap();
        assert_eq!(event.kind, RunEventKind::Blocked);
        assert_eq!(event.reason.as_deref(), Some("Usage limit reached"));
        assert_eq!(event.at, f.began + 1_000);

        // Collecting again with nothing new adds no event.
        collect_and_store(&f.store, &f.id, &f.settings).unwrap();
        assert_eq!(f.run().events.len(), 1);

        let resumed = f.activity(3_600_000);
        assert_eq!(resumed.status, RunStatus::Running);
        assert_eq!(resumed.blocking_reason, None);
        let event = resumed.events.last().unwrap();
        assert_eq!(event.kind, RunEventKind::Resumed);
        assert_eq!(event.at, f.began + 3_600_000);
        assert_eq!(resumed.events.len(), 2);
        assert_eq!(
            resumed.usage.unwrap().pauses[0].resumed_at,
            Some(f.began + 3_600_000)
        );
    }

    #[test]
    fn watching_collects_working_and_pause_blocked_runs_but_not_done_or_long_idle_ones() {
        let mut f = Fixture::new();
        f.limit(1_000);
        // A pause seen only by the watcher: wipe the block, keep the log, and let the watcher find it.
        f.lines.push(
            json!({"type": "assistant", "timestamp": iso_timestamp(f.began + 3_600_000), "message": {"content": "x"}})
                .to_string(),
        );
        f.world.write("claude/projects/-w/S1.jsonl", &f.lines);
        let now = crate::run::updated_now().unwrap();
        collect_active(&f.store, &f.settings, now).unwrap();
        assert_eq!(f.run().status, RunStatus::Running, "resumed with no update");

        // Long idle: the watcher leaves a run alone.
        let mut idle = Fixture::new();
        idle.lines.push(
            json!({"type": "assistant", "isApiErrorMessage": true, "error": "rate_limit",
                "timestamp": iso_timestamp(idle.began + 1_000), "message": {"model": "<synthetic>", "usage": {}}}).to_string(),
        );
        idle.world.write("claude/projects/-w/S1.jsonl", &idle.lines);
        let later = crate::run::updated_now().unwrap() + 8 * 24 * 60 * 60 * 1000;
        collect_active(&idle.store, &idle.settings, later).unwrap();
        assert_eq!(idle.run().status, RunStatus::Running);
        let now = crate::run::updated_now().unwrap();
        collect_active(&idle.store, &idle.settings, now).unwrap();
        assert_eq!(idle.run().status, RunStatus::Blocked);

        // Done runs are not collected at all.
        let done = Fixture::new();
        done.store.set_stage(&done.id, Stage::Finish);
        done.agent_update(RunStatus::Done, None);
        collect_active(&done.store, &done.settings, now).unwrap();
        assert!(done.run().usage.is_none());
    }
}

// Making a file unreadable is a Unix permission trick; it is skipped when the tests run as root.
// --- Codex attribution ------------------------------------------------------------------------------

// --- cost of Codex usage ------------------------------------------------------------------------------

fn priced_row(model: &str, tokens: serde_json::Value, long: serde_json::Value) -> UsageRow {
    serde_json::from_value(
        json!({"stage": "build", "model": model, "long_context": long, "tokens": 0}),
    )
    .map(|mut row: UsageRow| {
        row.tokens = serde_json::from_value(tokens).unwrap();
        row
    })
    .unwrap()
}

fn cost_of(row: &UsageRow) -> Option<f64> {
    crate::cost::row_cost(row, crate::cost::PRICES)
}

#[test]
fn long_context_tokens_are_priced_at_the_long_rates_and_the_rest_at_the_short_rates() {
    // gpt-5.5: short 5/0.5/30, long 10/1/45 per 1M.
    let all_long = priced_row(
        "gpt-5.5",
        json!({"input": 100_000, "cache_read": 200_000, "output": 1_000}),
        json!({"input": 100_000, "cache_read": 200_000, "output": 1_000}),
    );
    // 0.1M*10 + 0.2M*1 + 0.001M*45 = 1.0 + 0.2 + 0.045
    assert!((cost_of(&all_long).unwrap() - 1.245).abs() < 1e-9);
    // Add a short request of 10,000 input, 20,000 cached, 500 output: 0.05 + 0.01 + 0.015 = 0.075
    let mixed = priced_row(
        "gpt-5.5",
        json!({"input": 110_000, "cache_read": 220_000, "output": 1_500}),
        json!({"input": 100_000, "cache_read": 200_000, "output": 1_000}),
    );
    assert!((cost_of(&mixed).unwrap() - 1.32).abs() < 1e-9);
}

#[test]
fn codex_models_without_an_official_price_stay_unknown_and_make_totals_partial() {
    let known = priced_row(
        "gpt-5.2",
        json!({"input": 1_000_000, "output": 0}),
        json!({}),
    );
    let unknown = priced_row(
        "codex-auto-review",
        json!({"input": 10, "output": 10}),
        json!({}),
    );
    for model in [
        "gpt-5-codex",
        "gpt-5.1-codex-max",
        "gpt-5.2-codex",
        "codex-mini-latest",
    ] {
        assert_eq!(
            cost_of(&priced_row(
                model,
                json!({"input": 1, "output": 1}),
                json!({})
            )),
            None
        );
    }
    assert_eq!(cost_of(&unknown), None);
    let total = crate::cost::total([&known, &unknown], crate::cost::PRICES);
    assert!((total.usd.unwrap() - 1.75).abs() < 1e-9);
    assert!(total.partial);
    assert_eq!(total.unpriced_models, ["codex-auto-review"]);
}

#[test]
fn completed_routed_stage_with_only_coordinator_usage_adds_no_model_warning() {
    let world = World::new();
    let path = world.dir.path().join("runs.json");
    fs::write(&path, json!([{ "id": RUN_A, "title": "task", "gates": [], "project": "/p", "stage": "build", "status": "running",
        "timeline": [{"at": T0, "stage": "build"}],
        "model_routing": {"agent": "claude", "stages": {"build": {"model": "sonnet"}}} }]).to_string()).unwrap();
    let store = crate::run::RunStore::open(&path).unwrap();
    world.write(
        "claude/projects/-w/S1.jsonl",
        &[
            mention(RUN_A),
            claude_line(
                "S1",
                10,
                "m1",
                "r1",
                "claude-opus-5",
                (10, 0, 0, 1),
                0,
                false,
            ),
            claude_line(
                "S1",
                11,
                "m2",
                "r2",
                "claude-sonnet-5",
                (10, 0, 0, 1),
                0,
                false,
            ),
        ],
    );
    let settings = UsageSettings {
        enabled: true,
        roots: world.roots(),
        budget: Duration::from_secs(10),
        settle: Vec::new(),
    };
    collect_and_store(&store, RUN_A, &settings).unwrap();
    assert!(store.get(RUN_A).unwrap().unwrap().run_reports.is_empty());
    store.set_stage(RUN_A, Stage::Check);
    store
        .update(
            RUN_A,
            crate::run::UpdateRun::new(crate::run::RunStatus::Running),
        )
        .unwrap();
    collect_and_store(&store, RUN_A, &settings).unwrap();
    collect_and_store(&store, RUN_A, &settings).unwrap();
    let reopened = crate::run::RunStore::open(&path)
        .unwrap()
        .get(RUN_A)
        .unwrap()
        .unwrap();
    assert!(reopened.run_reports.is_empty());
}
