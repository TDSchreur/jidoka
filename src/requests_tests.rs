//! Fixture-based tests: small Claude Code logs in a temporary folder, read through the command's own
//! analysis. Nothing here reads a real user's logs.

use super::*;
use crate::{request_logs::Category, run::Run, usage_logs::Roots};
use serde_json::{Value, json};
use std::{fs, time::Duration};
use tempfile::TempDir;

/// 2026-11-05T10:00:00Z.
const T0: u64 = 1_793_872_800_000;
const RUN_A: &str = "1793872800000000001";
fn iso(offset_seconds: u64) -> String {
    crate::usage::iso_timestamp(T0 + offset_seconds * 1000)
}

fn run(id: &str, start: u64, end: Option<u64>, sessions: &[&str]) -> Run {
    serde_json::from_value(json!({
        "id": id, "title": "A title", "gates": [], "project": "/p", "stage": "build", "status": "running",
        "timeline": [{"at": T0 + start * 1000, "stage": "build", "rework": null, "ticket": null}],
        "finished_at": end.map(|s| T0 + s * 1000), "sessions": sessions,
    }))
    .unwrap()
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

    /// Writes the log of a session.
    fn session(&self, id: &str, lines: &[String]) {
        let path = self
            .dir
            .path()
            .join(format!("claude/projects/slug/{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    /// Writes any file of the fixture home.
    fn file(&self, relative: &str, lines: &[String]) {
        let path = self.dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    fn analyze(&self, target: &Run, runs: &[Run]) -> RunRequests {
        analyze(
            target,
            runs,
            &self.roots(),
            T0 + 10_000_000,
            Instant::now() + Duration::from_secs(30),
        )
    }
}

fn bash(command: &str) -> Value {
    json!({"type": "tool_use", "id": "t", "name": "Bash", "input": {"command": command}})
}

fn tool(name: &str) -> Value {
    json!({"type": "tool_use", "id": "t", "name": name, "input": {"file_path": "/SECRET/path"}})
}

fn text() -> Value {
    json!({"type": "text", "text": "SECRET conversation text"})
}

/// One line of a response: Claude Code writes a response once per content block.
fn line(at: u64, message: &str, block: Value, usage: (u64, u64, u64, u64)) -> String {
    json!({
        "type": "assistant", "timestamp": iso(at), "requestId": format!("req-{message}"), "isSidechain": false,
        "message": {"id": message, "model": "claude-test", "role": "assistant", "content": [block],
            "usage": {"input_tokens": usage.0, "cache_read_input_tokens": usage.1,
                      "cache_creation_input_tokens": usage.2, "output_tokens": usage.3}}
    })
    .to_string()
}

fn one(at: u64, message: &str, block: Value) -> String {
    line(at, message, block, (10, 100, 5, 20))
}

fn rows(result: &RunRequests) -> Vec<(Category, u64)> {
    result
        .categories
        .iter()
        .map(|row| (row.category, row.counts.requests))
        .collect()
}

/// The category of a lone request that runs `command`.
fn category_of(command: &str) -> Category {
    let world = World::new();
    world.session("s", &[one(10, "m1", bash(command))]);
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.categories.len(), 1, "{command}: {result:?}");
    result.categories[0].category
}

#[test]
fn a_request_counts_once_in_one_category_with_its_largest_tokens() {
    let world = World::new();
    world.session(
        "s",
        &[
            // One response written for two content blocks, the second with higher counts.
            line(10, "m1", bash("git commit -m x"), (10, 100, 5, 7)),
            line(10, "m1", bash("cargo test"), (10, 100, 5, 20)),
            one(20, "m2", text()),
            one(30, "m3", tool("Read")),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));

    // A commit and a verification in one request: the commit wins by priority.
    assert_eq!(
        rows(&result),
        [
            (Category::Commit, 1),
            (Category::ReadingEditingAnalysis, 1),
            (Category::TextOnly, 1)
        ]
    );
    assert_eq!(result.total.requests, 3);
    let commit = &result.categories[0].counts.tokens;
    assert_eq!(
        (
            commit.input,
            commit.cache_read,
            commit.cache_write,
            commit.output
        ),
        (Some(10), Some(100), Some(5), Some(20))
    );
    assert_eq!(result.total.tokens.output, Some(60));
}

#[test]
fn each_command_lands_in_the_category_the_rules_give_it() {
    for (command, expected) in [
        (
            "git worktree add -b x ../y origin/develop",
            Category::Workspace,
        ),
        (
            "jidoka run start \"A title\" --agent claude",
            Category::Workspace,
        ),
        ("git commit -m \"Add it\"", Category::Commit),
        ("git add -A && git commit -m x", Category::Commit),
        ("git push -u origin x", Category::PrCreationAndPush),
        (
            "gh pr create --title x --body y",
            Category::PrCreationAndPush,
        ),
        ("git fetch origin develop", Category::PrSynchronization),
        ("git merge origin/develop", Category::PrSynchronization),
        (
            "gh pr view --json mergeStateStatus",
            Category::PrSynchronization,
        ),
        ("gh pr checks 12 --watch", Category::CiWatch),
        ("gh run watch 99", Category::CiWatch),
        (
            "az repos pr create --source-branch x --target-branch develop --title t",
            Category::PrCreationAndPush,
        ),
        (
            "az repos pr update --id 7 --description d",
            Category::PrCreationAndPush,
        ),
        ("az pipelines runs list --branch x", Category::CiWatch),
        ("az pipelines runs show --id 42", Category::CiWatch),
        ("az repos pr policy list --id 7", Category::CiWatch),
        (
            "az devops invoke --area git --resource pullRequestStatuses --route-parameters project=p repositoryId=r pullRequestId=7",
            Category::CiWatch,
        ),
        (
            "az devops invoke --area git --resource pullRequestIterationStatuses --route-parameters iterationId=2",
            Category::CiWatch,
        ),
        (
            "az boards work-item create --title t --type Task",
            Category::ReadingEditingAnalysis,
        ),
        ("az account show", Category::ReadingEditingAnalysis),
        (
            "az devops invoke --area wit --resource workitems",
            Category::ReadingEditingAnalysis,
        ),
        ("az repos pr show --id 7", Category::ReadingEditingAnalysis),
        (
            "az devops invoke --resource pullRequests --query Statuses",
            Category::ReadingEditingAnalysis,
        ),
        ("scripts/verify.sh", Category::Verification),
        (
            "cd /repo && cargo clippy --all-targets -- -D warnings",
            Category::Verification,
        ),
        ("FOO=1 cargo test", Category::Verification),
        ("jidoka run inspect 123", Category::ContinuationCheck),
        ("jidoka run check 123 --json", Category::ContinuationCheck),
        ("git worktree list --porcelain", Category::ContinuationCheck),
        (
            "jidoka run update 1 running --activity x",
            Category::StandaloneReporting,
        ),
        (
            "cd /repo && jidoka run update 1 running",
            Category::StandaloneReporting,
        ),
        ("ls -la", Category::ReadingEditingAnalysis),
        (
            "jidoka run update 1 running && ls",
            Category::ReadingEditingAnalysis,
        ),
    ] {
        assert_eq!(category_of(command), expected, "{command}");
    }
}

#[test]
fn a_session_claims_a_run_only_through_an_executed_update_command() {
    let world = World::new();
    // Executed `jidoka run update <id>`: claims.
    world.session(
        "executed",
        &[
            one(
                10,
                "e1",
                bash(&format!("jidoka run update {RUN_A} build running")),
            ),
            one(20, "e2", text()),
        ],
    );
    // The id appears only in a tool result, a prompt and an inspect command: no claim.
    world.session(
        "mentioned",
        &[
            json!({"type": "user", "timestamp": iso(5), "message": {"content": [
                {"type": "tool_result", "content": format!("jidoka run update {RUN_A} build running")}]}})
            .to_string(),
            json!({"type": "user", "timestamp": iso(6), "message": {"content": format!("jidoka run update {RUN_A}")}}).to_string(),
            one(10, "n1", bash(&format!("jidoka run inspect {RUN_A}"))),
            one(20, "n2", bash(&format!("cat notes.md # jidoka run stats {RUN_A}"))),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &[]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.sessions, ["executed"]);
    assert_eq!(result.total.requests, 2);
}

#[test]
fn requests_outside_the_run_are_not_counted() {
    let world = World::new();
    world.session(
        "s",
        &[
            one(5, "early", text()),
            one(50, "inside", text()),
            one(500, "late", text()),
        ],
    );
    let target = run(RUN_A, 10, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.total.requests, 1);
}

#[test]
fn the_table_and_the_json_show_the_same_figures_and_none_of_the_log_text() {
    let world = World::new();
    world.session(
        "s",
        &[
            one(10, "m1", bash("git commit -m \"SECRET message\"")),
            one(20, "m2", text()),
            one(30, "m3", tool("Edit")),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));

    let table = render_run(&result);
    let json = serde_json::to_string(&result).unwrap();
    for output in [&table, &json] {
        assert!(!output.contains("SECRET"), "{output}");
    }
    let commit_row = table.lines().find(|l| l.contains("commit")).expect(&table);
    assert!(
        commit_row.contains('1') && commit_row.contains("33.3%"),
        "{commit_row}"
    );
    assert!(table.contains("text only") && table.contains("reading, editing, analysis"));
    let parsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["total"]["requests"], 3);
    assert_eq!(parsed["categories"][0]["category"], "commit");
    assert_eq!(parsed["categories"][0]["input"], 10);
}

/// A line as a subagent writes it.
fn sidechain(line: String) -> String {
    let mut value: Value = serde_json::from_str(&line).unwrap();
    value["isSidechain"] = json!(true);
    value.to_string()
}

fn sources(result: &RunRequests) -> Vec<(Category, bool, u64)> {
    result
        .categories
        .iter()
        .map(|row| (row.category, row.subagent, row.counts.requests))
        .collect()
}

#[test]
fn subagent_requests_are_categorized_by_their_own_commands_and_shown_apart() {
    let world = World::new();
    world.session("s", &[one(10, "m1", bash("cargo test"))]);
    world.file(
        "claude/projects/slug/s/subagents/agent-1.jsonl",
        &[
            sidechain(one(20, "sub1", bash("git commit -m x"))),
            sidechain(one(25, "sub2", text())),
            // The same response as the coordinator's: counted once.
            sidechain(one(10, "m1", bash("cargo test"))),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));

    assert_eq!(
        sources(&result),
        [
            (Category::Commit, true, 1),
            (Category::Verification, false, 1),
            (Category::TextOnly, true, 1)
        ]
    );
    assert_eq!(result.total.requests, 3);
    assert_eq!(result.subagents.requests, 2);
    assert!(
        result
            .notes
            .iter()
            .any(|n| n.contains("Subagent requests are included")),
        "{:?}",
        result.notes
    );
    assert!(!result.notes.iter().any(|n| n.contains("coordinator-only")));
    let table = render_run(&result);
    assert!(table.contains("commit (subagents)"), "{table}");
    assert!(table.contains("of which subagents"), "{table}");
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["categories"][0]["subagent"], true);
    assert_eq!(json["subagents"]["requests"], 2);
}

fn copilot_message(at: u64, call: &str, calls: &[Value]) -> String {
    json!({"type": "assistant.message", "timestamp": iso(at), "data": {"messageId": "m", "model": "gpt-test",
        "content": "", "toolRequests": calls, "turnId": "0", "apiCallId": call}})
    .to_string()
}

fn copilot_call(id: &str, name: &str, arguments: Value) -> Value {
    json!({"toolCallId": id, "name": name, "arguments": arguments, "type": "function"})
}

fn copilot_result(at: u64, id: &str, content: &str) -> String {
    json!({"type": "tool.execution_complete", "timestamp": iso(at), "data": {"toolCallId": id, "success": true,
        "result": {"content": content, "detailedContent": "not counted, the model never sees it"}}})
    .to_string()
}

#[test]
fn copilot_requests_are_categorized_by_their_tool_calls_and_subagents_shown_apart() {
    let world = World::new();
    world.file(
        "copilot/session-state/c1/events.jsonl",
        &[
            copilot_message(
                10,
                "a1",
                &[copilot_call(
                    "t1",
                    "bash",
                    json!({"command": "cargo test", "description": "d"}),
                )],
            ),
            copilot_usage(11, "a1", false),
            copilot_result(12, "t1", "ok 12 tests"),
            copilot_message(
                20,
                "a2",
                &[
                    copilot_call("t2", "view", json!({"path": "/p/src/main.rs"})),
                    copilot_call(
                        "t3",
                        "task",
                        json!({"name": "probe", "agent_type": "explore", "prompt": "x"}),
                    ),
                ],
            ),
            copilot_usage(21, "a2", false),
            copilot_result(22, "t2", "fn main() {}"),
            copilot_result(23, "t3", "done"),
            copilot_message(
                30,
                "a3",
                &[copilot_call(
                    "t4",
                    "bash",
                    json!({"command": format!("jidoka run update {RUN_A} build running")}),
                )],
            ),
            copilot_usage(31, "a3", false),
            copilot_result(32, "t4", ""),
            copilot_message(40, "a4", &[]),
            copilot_usage(41, "a4", false),
            // A subagent request, with its own commands.
            copilot_message(
                50,
                "s1",
                &[copilot_call(
                    "t5",
                    "bash",
                    json!({"command": "git commit -m x"}),
                )],
            ),
            copilot_usage(51, "s1", true),
            copilot_result(52, "t5", "[main abc] x"),
            // The result of this call is not in the log, so its output size is unknown.
            copilot_message(
                60,
                "a5",
                &[copilot_call(
                    "t6",
                    "bash",
                    json!({"command": "cargo build"}),
                )],
            ),
            copilot_usage(61, "a5", false),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["c1"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));

    assert_eq!(
        sources(&result),
        [
            (Category::Commit, true, 1),
            (Category::Verification, false, 2),
            (Category::StandaloneReporting, false, 1),
            (Category::ReadingEditingAnalysis, false, 1),
            (Category::TextOnly, false, 1),
        ]
    );
    let sizes: Vec<_> = result
        .categories
        .iter()
        .map(|row| row.counts.tool_output_bytes)
        .collect();
    // verification: "ok 12 tests" only, because the second request's result is missing.
    assert_eq!(sizes, [Some(12), Some(11), Some(0), Some(16), Some(0)]);
    assert_eq!(result.subagents.requests, 1);
    assert!(result.coordinator_only_sessions.is_empty());
    assert!(
        !result
            .notes
            .iter()
            .any(|n| n.contains("no matching assistant message"))
    );
}

fn codex_usage(at: u64, response: &str) -> String {
    json!({"type": "token_usage_record", "timestamp": iso(at), "payload": {"response_id": response, "turn_id": "turn-1",
        "usage": {"input_tokens": 100, "cached_input_tokens": 40, "cache_write_input_tokens": 0,
                  "output_tokens": 20, "reasoning_output_tokens": 5, "total_tokens": 120}}})
    .to_string()
}

fn copilot_usage(at: u64, call: &str, sub: bool) -> String {
    json!({"type": "session.usage_record", "timestamp": iso(at), "data": {"usage": {"model": "gpt-test", "apiCallId": call,
        "inputTokens": 50, "cacheReadTokens": 10, "cacheWriteTokens": 0, "outputTokens": 7,
        "reasoningTokens": 3, "initiator": if sub {"sub-agent"} else {"user"}}}})
    .to_string()
}

// --- the pooled view --------------------------------------------------------------------------------

use crate::requests_pool::{PoolOptions, pool, render_pool};

fn run_id(n: u64) -> String {
    format!("17938728000000000{n:02}")
}

/// A run with usage that is `tracked`, `partial` or missing, done or still running.
fn pooled_run(n: u64, done: bool, usage: &str) -> Run {
    let mut value = json!({
        "id": run_id(n), "title": format!("Run {n}"), "gates": [], "project": "/p", "stage": "finish",
        "status": if done { "done" } else { "running" },
        "timeline": [{"at": T0, "stage": "build", "rework": null, "ticket": null}],
        "finished_at": done.then_some(T0 + 100_000), "sessions": [format!("s{n}")],
    });
    match usage {
        "tracked" => value["usage"] = json!({"status": "tracked", "collected_at": 0}),
        "partial" => {
            value["usage"] =
                json!({"status": "partial", "notes": ["a log was malformed"], "collected_at": 0})
        }
        _ => {}
    }
    serde_json::from_value(value).unwrap()
}

/// A session of `commits` commit requests and one text request.
fn pooled_session(world: &World, n: u64, commits: u64) {
    let mut lines: Vec<String> = (0..commits)
        .map(|i| one(10 + i, &format!("c{n}-{i}"), bash("git commit -m x")))
        .collect();
    lines.push(one(50, &format!("t{n}"), text()));
    world.session(&format!("s{n}"), &lines);
}

fn pooled_world() -> (World, Vec<Run>) {
    let world = World::new();
    let mut runs = Vec::new();
    for (n, commits) in [0, 1, 2, 3, 10].into_iter().enumerate() {
        let n = n as u64 + 1;
        pooled_session(&world, n, commits);
        runs.push(pooled_run(n, true, "tracked"));
    }
    (world, runs)
}

fn pooled(
    world: &World,
    runs: &[Run],
    options: &PoolOptions,
) -> crate::requests_pool::PooledRequests {
    pool(
        runs,
        &world.roots(),
        T0 + 10_000_000,
        Instant::now() + Duration::from_secs(30),
        options,
    )
}

#[test]
fn the_pooled_view_shows_spread_and_shares_over_the_completed_runs() {
    let (world, runs) = pooled_world();
    let result = pooled(&world, &runs, &PoolOptions::default());

    assert_eq!(result.included.len(), 5);
    let commit = result
        .categories
        .iter()
        .find(|c| c.category == Category::Commit)
        .unwrap();
    // Requests per run: 0, 1, 2, 3, 10.
    assert_eq!(
        (
            commit.median,
            commit.p25,
            commit.p75,
            commit.min,
            commit.max
        ),
        (2.0, 1.0, 3.0, 0, 10)
    );
    assert_eq!(commit.counts.requests, 16);
    assert_eq!(commit.counts.tokens.input, Some(160));
    let text_only = result
        .categories
        .iter()
        .find(|c| c.category == Category::TextOnly)
        .unwrap();
    assert_eq!(
        (text_only.median, text_only.min, text_only.max),
        (1.0, 1, 1)
    );
    assert_eq!(result.total.requests, 21);
    // 16 of 21 requests, and as many of the fresh tokens because every request is the same size.
    assert!((commit.request_share - 16.0 / 21.0).abs() < 1e-9);
    assert!((commit.fresh_share.unwrap() - 16.0 / 21.0).abs() < 1e-9);

    let table = render_pool(&result);
    let words: Vec<&str> = table
        .lines()
        .find(|l| l.trim_start().starts_with("commit "))
        .expect(&table)
        .split_whitespace()
        .collect();
    assert_eq!(&words[..6], ["commit", "2", "1", "3", "0", "10"], "{table}");
    assert!(table.contains("5 runs"), "{table}");

    let json = serde_json::to_value(&result).unwrap();
    let row = json["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["category"] == "commit")
        .unwrap();
    assert_eq!(row["median"], 2.0);
    assert_eq!(row["max"], 10);
    assert_eq!(row["requests"], 16);
}

// --- which run a command updates, and which subagent logs count ---------------------------------------

// --- quotes and wrapper words around an update command -----------------------------------------------

// --- comments, here-strings and wrappers with options ------------------------------------------------

// --- commands as agents write them -------------------------------------------------------------------

// --- Codex commands ---------------------------------------------------------------------------------

const CODEX_FILE: &str = "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-abc.jsonl";

fn codex_meta(id: &str, root: Option<&str>) -> String {
    let mut payload = json!({"id": id, "source": "cli"});
    if let Some(root) = root {
        payload["session_id"] = json!(root);
        payload["source"] = json!({"subagent": {"thread_spawn": {"parent_thread_id": root}}});
    }
    json!({"type": "session_meta", "timestamp": iso(0), "payload": payload}).to_string()
}

/// A `custom_tool_call` whose JavaScript snippet is `snippet`.
fn codex_exec(call: &str, snippet: &str) -> String {
    json!({"type": "response_item", "timestamp": iso(5), "payload": {
        "type": "custom_tool_call", "status": "completed", "call_id": call, "name": "exec", "input": snippet}})
    .to_string()
}

/// An `exec` call that runs these commands, as a JSON string literal each, in one `Promise.allSettled`.
fn codex_exec_commands(call: &str, commands: &[&str]) -> String {
    let calls: Vec<String> = commands
        .iter()
        .map(|c| {
            format!(
                "tools.exec_command({{cmd:{},workdir:\"/w\",max_output_tokens:2000}})",
                serde_json::to_string(c).unwrap()
            )
        })
        .collect();
    codex_exec(
        call,
        &format!(
            "const r=await Promise.allSettled([{}]);text(JSON.stringify(r));",
            calls.join(",")
        ),
    )
}

fn codex_function_call(call: &str, name: &str) -> String {
    json!({"type": "response_item", "timestamp": iso(5), "payload": {
        "type": "function_call", "status": "completed", "call_id": call, "name": name, "arguments": "{}"}})
    .to_string()
}

#[test]
fn codex_responses_get_the_category_of_their_commands_and_other_tool_calls() {
    let world = World::new();
    world.file(
        CODEX_FILE,
        &[
            codex_meta("abc", None),
            codex_exec_commands("a1", &["git commit -m x"]),
            codex_usage(10, "r1"),
            // Two commands in one snippet: a verification and a push; the push wins by priority.
            codex_exec_commands("a2", &["cargo test", "git push origin x"]),
            codex_usage(20, "r2"),
            codex_exec(
                "a3",
                "text(await tools.apply_patch(\"*** Begin Patch\\n*** End Patch\"))",
            ),
            codex_usage(30, "r3"),
            codex_function_call("a4", "spawn_agent"),
            codex_usage(40, "r4"),
            codex_usage(50, "r5"),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["abc"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(
        rows(&result),
        [
            (Category::Commit, 1),
            (Category::PrCreationAndPush, 1),
            (Category::ReadingEditingAnalysis, 2),
            (Category::TextOnly, 1)
        ]
    );
    assert_eq!(result.several_categories, 1);
    assert_eq!(result.total.requests, 5);
}

#[test]
fn a_codex_subagent_rollout_is_categorized_and_shown_on_its_own_lines() {
    let world = World::new();
    world.file(
        CODEX_FILE,
        &[
            codex_meta("abc", None),
            codex_exec_commands("a1", &["cargo test"]),
            codex_usage(10, "r1"),
        ],
    );
    world.file(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-30-sub1.jsonl",
        &[
            codex_meta("sub1", Some("abc")),
            codex_exec_commands("s1", &["git log -1"]),
            codex_usage(20, "s-r1"),
            codex_usage(30, "s-r2"),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["abc"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(
        sources(&result),
        [
            (Category::Verification, false, 1),
            (Category::ReadingEditingAnalysis, true, 1),
            (Category::TextOnly, true, 1)
        ]
    );
    assert_eq!(result.subagents.requests, 2);
    assert!(result.coordinator_only_sessions.is_empty());
}
