//! One fixture per agent, read through both reports: Token usage and the request analysis must agree on the
//! responses they count and on their tokens, whichever agent wrote the log.

use super::*;
use crate::{
    run::Run,
    usage::{self, requests},
};
use serde_json::json;
use std::time::Duration;

/// 2026-11-05T10:00:00Z.
const T0: u64 = 1_793_872_800_000;
const RUN: &str = "1793872800000000001";

fn iso(offset_seconds: u64) -> String {
    crate::timestamps::iso(T0 + offset_seconds * 1000)
}

fn command() -> String {
    format!("jidoka run update {RUN} build running")
}

/// Two responses; the first one runs a command that names the run.
fn claude_log() -> Vec<String> {
    let response = |at: u64, id: &str, content: serde_json::Value, usage: serde_json::Value| {
        json!({"type": "assistant", "timestamp": iso(at), "requestId": format!("req-{id}"), "isSidechain": false,
            "message": {"id": id, "model": "claude-test", "role": "assistant", "content": [content], "usage": usage}})
        .to_string()
    };
    vec![
        response(
            10,
            "m1",
            json!({"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": command()}}),
            json!({"input_tokens": 100, "cache_read_input_tokens": 50, "cache_creation_input_tokens": 10, "output_tokens": 20}),
        ),
        response(
            20,
            "m2",
            json!({"type": "text", "text": "done"}),
            json!({"input_tokens": 200, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0, "output_tokens": 30}),
        ),
    ]
}

fn codex_log() -> Vec<String> {
    let record = |at: u64, id: &str, input: u64, cached: u64, output: u64| {
        json!({"type": "token_usage_record", "timestamp": iso(at), "payload": {"response_id": id, "turn_id": "turn-1",
            "usage": {"input_tokens": input, "cached_input_tokens": cached, "output_tokens": output}}})
        .to_string()
    };
    vec![
        json!({"type": "session_meta", "timestamp": iso(0), "payload": {"id": "cdx1", "session_id": "cdx1", "source": "cli"}}).to_string(),
        json!({"type": "turn_context", "timestamp": iso(0), "payload": {"model": "gpt-test", "turn_id": "turn-1"}}).to_string(),
        json!({"type": "response_item", "timestamp": iso(5), "payload": {"type": "custom_tool_call", "name": "exec", "call_id": "c1",
            "input": format!("tools.exec_command({{cmd: \"{}\"}})", command())}}).to_string(),
        record(10, "r1", 150, 50, 20),
        record(20, "r2", 200, 0, 30),
    ]
}

fn copilot_log() -> Vec<String> {
    let record = |at: u64, call: &str, input: u64, read: u64, output: u64| {
        json!({"type": "session.usage_record", "timestamp": iso(at), "data": {"usage": {"model": "gpt-test", "apiCallId": call,
            "inputTokens": input, "cacheReadTokens": read, "outputTokens": output, "initiator": "agent"}}})
        .to_string()
    };
    vec![
        json!({"type": "assistant.message", "timestamp": iso(9), "data": {"apiCallId": "a1", "content": "", "toolRequests": [
            {"toolCallId": "t1", "name": "bash", "arguments": {"command": command()}}]}}).to_string(),
        record(10, "a1", 160, 50, 20),
        record(20, "a2", 200, 0, 30),
    ]
}

#[test]
fn usage_and_requests_count_the_same_responses_and_tokens_for_every_agent() {
    let logs = [
        ("Claude Code", "claude/projects/slug/s1.jsonl", claude_log()),
        (
            "Codex",
            "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-cdx1.jsonl",
            codex_log(),
        ),
        (
            "Copilot",
            "copilot/session-state/cop1/events.jsonl",
            copilot_log(),
        ),
    ];
    for (agent, file, lines) in logs {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        let roots = Roots::from_homes(|agent| home.path().join(agent.name()));
        let target: Run = serde_json::from_value(json!({
            "id": RUN, "title": "t", "gates": [], "project": "/p", "stage": "build", "status": "running",
            "timeline": [{"at": T0, "stage": "build", "rework": null, "ticket": null}],
            "finished_at": T0 + 100_000, "sessions": [],
        }))
        .unwrap();
        let runs = std::slice::from_ref(&target);
        let (now, deadline) = (T0 + 1_000_000, Instant::now() + Duration::from_secs(30));

        let usage = usage::collection::collect(&target, runs, &roots, now, deadline);
        let requests = requests::analyze(&target, runs, &roots, now, deadline);

        let mut used = Tokens::default();
        for row in &usage.rows {
            used.add(&row.tokens);
        }
        assert_eq!(used, requests.total.tokens, "{agent}: tokens");
        assert_eq!(requests.total.requests, 2, "{agent}: requests");
        let window = RunWindow::new(&target, runs, T0);
        let read = read_sessions(&roots, &window, deadline, ReadLimits::default());
        let entries: usize = read.sessions.iter().map(|s| s.responses.len()).sum();
        assert_eq!(entries, 2, "{agent}: responses read");
        assert_eq!(usage.sessions.len(), 1, "{agent}: sessions in usage");
        assert_eq!(requests.sessions.len(), 1, "{agent}: sessions in requests");
        assert_eq!(usage.sessions, requests.sessions, "{agent}: same session");
    }
}

#[test]
fn copilot_claims_a_run_through_a_shell_variable_but_not_through_a_prompt() {
    let ids = HashSet::from([RUN]);
    let via_variable = json!({"type": "assistant.message", "timestamp": iso(1), "data": {"apiCallId": "c1", "content": "",
        "toolRequests": [{"toolCallId": "t1", "name": "bash",
            "arguments": {"command": format!("id={RUN} && jidoka run update $id define running")}}]}})
    .to_string();
    assert_eq!(
        copilot::copilot_claims(&via_variable, &ids),
        HashSet::from([RUN.to_string()])
    );
    let prompt_only =
        json!({"type": "user.message", "data": {"content": format!("Run: {RUN}")}}).to_string();
    assert!(copilot::copilot_claims(&prompt_only, &ids).is_empty());
}

#[test]
fn copilot_claims_a_run_through_the_coordinator_but_not_through_a_subagent() {
    let ids = HashSet::from([RUN]);
    let message = |call: &str| {
        json!({"type": "assistant.message", "timestamp": iso(1), "data": {"apiCallId": call, "content": "",
            "toolRequests": [{"toolCallId": format!("t-{call}"), "name": "bash",
                "arguments": {"command": format!("jidoka run stats {RUN}")}}]}})
        .to_string()
    };
    let record = |call: &str, initiator: &str| {
        json!({"type": "session.usage_record", "timestamp": iso(2), "data": {"usage": {"apiCallId": call,
            "model": "gpt-5", "inputTokens": 10, "outputTokens": 1, "initiator": initiator}}})
        .to_string()
    };
    let subagent = [message("c1"), record("c1", "sub-agent")].join("\n");
    assert!(copilot::copilot_claims(&subagent, &ids).is_empty());
    let coordinator = [message("c1"), record("c1", "user")].join("\n");
    assert_eq!(
        copilot::copilot_claims(&coordinator, &ids),
        HashSet::from([RUN.to_string()])
    );
}

/// Reads the logs of `file`s under a fresh home for a run that recorded `session`, and returns the
/// session the reader found: its claims and, per response in time order, whether it is subagent usage.
fn read_recorded(files: &[(&str, Vec<String>)], session: &str) -> (HashSet<String>, Vec<bool>) {
    let home = tempfile::tempdir().unwrap();
    for (file, lines) in files {
        let path = home.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, lines.join("\n") + "\n").unwrap();
    }
    let roots = Roots::from_homes(|agent| home.path().join(agent.name()));
    let target: Run = serde_json::from_value(json!({
        "id": RUN, "title": "t", "gates": [], "project": "/p", "stage": "build", "status": "running",
        "timeline": [{"at": T0, "stage": "build", "rework": null, "ticket": null}],
        "finished_at": T0 + 100_000, "sessions": [session],
    }))
    .unwrap();
    let window = RunWindow::new(&target, std::slice::from_ref(&target), T0);
    let read = read_sessions(
        &roots,
        &window,
        Instant::now() + Duration::from_secs(30),
        ReadLimits::default(),
    );
    assert_eq!(read.sessions.len(), 1);
    let found = &read.sessions[0];
    (
        found.claims.clone(),
        found.responses.iter().map(|r| r.subagent).collect(),
    )
}

#[test]
fn a_claude_subagent_naming_the_run_is_not_subagent_usage_and_does_not_claim() {
    let line = |at: u64, id: &str, command: Option<String>| {
        let content = match command {
            Some(command) => {
                json!([{"type": "tool_use", "id": format!("t-{id}"), "name": "Bash", "input": {"command": command}}])
            }
            None => json!([{"type": "text", "text": "ok"}]),
        };
        json!({"type": "assistant", "timestamp": iso(at), "requestId": format!("req-{id}"), "isSidechain": true,
            "message": {"id": id, "model": "claude-test", "role": "assistant", "content": content,
                "usage": {"input_tokens": 10, "output_tokens": 5}}})
        .to_string()
    };
    let main =
        json!({"type": "user", "timestamp": iso(1), "message": {"role": "user", "content": "go"}})
            .to_string();
    let (claims, subagent) = read_recorded(
        &[
            ("claude/projects/slug/s1.jsonl", vec![main]),
            (
                "claude/projects/slug/s1/subagents/agent-a.jsonl",
                vec![line(10, "a1", Some(command()))],
            ),
            (
                "claude/projects/slug/s1/subagents/agent-b.jsonl",
                vec![line(20, "b1", None)],
            ),
        ],
        "s1",
    );
    assert_eq!(subagent, [false, true]);
    assert!(claims.is_empty());
}

#[test]
fn a_codex_subagent_rollout_naming_the_run_is_not_subagent_usage_and_does_not_claim() {
    let rollout = |id: &str, at: u64, with_command: bool| {
        let mut lines = vec![
            json!({"type": "session_meta", "timestamp": iso(0), "payload": {"id": id, "session_id": "cdx1",
                "thread_source": "subagent", "source": {"subagent": {"thread_spawn": {"parent_thread_id": "cdx1"}}}}})
            .to_string(),
            json!({"type": "turn_context", "timestamp": iso(0), "payload": {"model": "gpt-test", "turn_id": "turn-1"}}).to_string(),
        ];
        if with_command {
            lines.push(json!({"type": "response_item", "timestamp": iso(at - 1), "payload": {"type": "custom_tool_call",
                "name": "exec", "call_id": "c1", "input": format!("tools.exec_command({{cmd: \"{}\"}})", command())}}).to_string());
        }
        lines.push(json!({"type": "token_usage_record", "timestamp": iso(at), "payload": {"response_id": format!("r-{id}"),
            "turn_id": "turn-1", "usage": {"input_tokens": 10, "cached_input_tokens": 0, "output_tokens": 5}}}).to_string());
        lines
    };
    let (claims, subagent) = read_recorded(
        &[
            (
                "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-sub-a.jsonl",
                rollout("sub-a", 10, true),
            ),
            (
                "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-sub-b.jsonl",
                rollout("sub-b", 20, false),
            ),
        ],
        "cdx1",
    );
    assert_eq!(subagent, [false, true]);
    assert!(claims.is_empty());
}

#[test]
fn a_copilot_sub_agent_naming_the_run_is_not_subagent_usage_and_does_not_claim() {
    let message = |call: &str, command: &str| {
        json!({"type": "assistant.message", "timestamp": iso(1), "data": {"apiCallId": call, "content": "",
            "toolRequests": [{"toolCallId": format!("t-{call}"), "name": "bash", "arguments": {"command": command}}]}})
        .to_string()
    };
    let record = |at: u64, call: &str, agent: &str| {
        json!({"type": "session.usage_record", "timestamp": iso(at), "agentId": agent, "data": {"usage": {"apiCallId": call,
            "model": "gpt-test", "inputTokens": 10, "outputTokens": 1, "initiator": "sub-agent"}}})
        .to_string()
    };
    let (claims, subagent) = read_recorded(
        &[(
            "copilot/session-state/cop1/events.jsonl",
            vec![
                message("c1", &command()),
                record(10, "c1", "agent-a"),
                // The same agent's later request names nothing, but it is still the coordinator's.
                message("c2", "ls"),
                record(20, "c2", "agent-a"),
                message("c3", "ls"),
                record(30, "c3", "agent-b"),
            ],
        )],
        "cop1",
    );
    assert_eq!(subagent, [false, false, true]);
    assert!(claims.is_empty());
}

#[test]
fn a_recorded_absolute_agent_home_wins_and_relative_or_unknown_ones_are_ignored() {
    let own = tempfile::tempdir().unwrap();
    let recorded = tempfile::tempdir().unwrap();
    let roots = Roots::from_home(own.path(), &|_| None);
    let mut target: Run = serde_json::from_value(json!({
        "id": RUN, "title": "t", "gates": [], "project": "/p", "stage": "build", "status": "running",
        "timeline": [], "sessions": [],
    }))
    .unwrap();
    target.agent_homes.extend([
        (
            "codex".to_string(),
            recorded.path().to_str().unwrap().to_string(),
        ),
        ("claude".to_string(), "relative/claude".to_string()),
        (
            "gemini".to_string(),
            recorded.path().to_str().unwrap().to_string(),
        ),
    ]);
    let for_run = roots.for_run(&target);
    assert_eq!(for_run.get(Agent::Codex), recorded.path().join("sessions"));
    assert_eq!(
        for_run.get(Agent::Claude),
        own.path().join(".claude").join("projects")
    );
    assert_eq!(
        for_run.get(Agent::Copilot),
        own.path().join(".copilot").join("session-state")
    );
}

#[test]
fn every_agent_has_exactly_one_log_reader() {
    for agent in Agent::ALL {
        let readers = READERS
            .iter()
            .filter(|reader| reader.agent() == agent)
            .count();
        assert_eq!(readers, 1, "{} needs exactly one log reader", agent.name());
    }
}
