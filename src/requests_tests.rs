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
const RUN_B: &str = "1793872800000000002";

fn iso(offset_seconds: u64) -> String {
    crate::usage::iso_timestamp(T0 + offset_seconds * 1000)
}

fn run(id: &str, start: u64, end: Option<u64>, sessions: &[&str]) -> Run {
    serde_json::from_value(json!({
        "id": id, "title": "A title", "project": "/p", "stage": "build", "status": "running",
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

/// A Bash call with its own id, so that a result can name it.
fn bash_call(id: &str, command: &str) -> Value {
    json!({"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}})
}

/// What a tool returned, as Claude Code logs it on a user line.
fn result(at: u64, id: &str, content: Value) -> String {
    json!({"type": "user", "timestamp": iso(at), "isSidechain": false,
        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": content}]}})
    .to_string()
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
            "factory run start \"A title\" --agent claude",
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
        ("factory run inspect 123", Category::ContinuationCheck),
        ("factory run check 123 --json", Category::ContinuationCheck),
        ("git worktree list --porcelain", Category::ContinuationCheck),
        (
            "factory run update 1 build running --activity x",
            Category::StandaloneReporting,
        ),
        (
            "cd /repo && factory run update 1 build running",
            Category::StandaloneReporting,
        ),
        ("ls -la", Category::ReadingEditingAnalysis),
        (
            "factory run update 1 build running && ls",
            Category::ReadingEditingAnalysis,
        ),
    ] {
        assert_eq!(category_of(command), expected, "{command}");
    }
}

#[test]
fn words_inside_a_heredoc_do_not_decide_the_category() {
    let command = "git commit -F - <<'EOF'\nRun cargo test and git push\nEOF";
    assert_eq!(category_of(command), Category::Commit);
}

#[test]
fn a_session_claims_a_run_only_through_an_executed_update_command() {
    let world = World::new();
    // Executed `factory run update <id>`: claims.
    world.session(
        "executed",
        &[
            one(
                10,
                "e1",
                bash(&format!("factory run update {RUN_A} build running")),
            ),
            one(20, "e2", text()),
        ],
    );
    // The id appears only in a tool result, a prompt and an inspect command: no claim.
    world.session(
        "mentioned",
        &[
            json!({"type": "user", "timestamp": iso(5), "message": {"content": [
                {"type": "tool_result", "content": format!("factory run update {RUN_A} build running")}]}})
            .to_string(),
            json!({"type": "user", "timestamp": iso(6), "message": {"content": format!("factory run update {RUN_A}")}}).to_string(),
            one(10, "n1", bash(&format!("factory run inspect {RUN_A}"))),
            one(20, "n2", bash(&format!("cat notes.md # factory run stats {RUN_A}"))),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &[]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.sessions, ["executed"]);
    assert_eq!(result.total.requests, 2);
}

#[test]
fn a_request_two_overlapping_runs_could_claim_is_in_neither() {
    let world = World::new();
    world.session(
        "shared",
        &[
            // Only run A is active.
            one(10, "a1", bash("cargo test")),
            // Both runs are active.
            one(60, "b1", bash("cargo test")),
            one(70, "b2", text()),
        ],
    );
    let a = run(RUN_A, 0, Some(200), &["shared"]);
    let b = run(RUN_B, 50, Some(200), &["shared"]);
    let runs = [a.clone(), b.clone()];

    let first = world.analyze(&a, &runs);
    assert_eq!(first.total.requests, 1);
    assert_eq!(first.ambiguous.requests, 2);
    assert!(
        first.notes.iter().any(|n| n.contains("another run")),
        "{:?}",
        first.notes
    );

    let second = world.analyze(&b, &runs);
    assert_eq!(second.total.requests, 0);
    assert_eq!(second.ambiguous.requests, 2);
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
fn a_run_no_log_names_says_why() {
    let world = World::new();
    world.session("other", &[one(10, "m", text())]);
    let target = run(RUN_A, 0, Some(100), &[]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.total.requests, 0);
    assert!(
        result.notes.iter().any(|n| n.contains("no session log")),
        "{:?}",
        result.notes
    );
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

#[test]
fn chained_reporting_is_a_count_beside_the_categories() {
    let world = World::new();
    world.session(
        "s",
        &[
            one(
                10,
                "m1",
                bash("cargo test && factory run update 1 build running"),
            ),
            one(20, "m2", bash("factory run update 1 build running")),
            one(30, "m3", bash("ls; factory run update 1 build running")),
            one(40, "m4", bash("cargo test")),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));

    assert_eq!(
        rows(&result),
        [
            (Category::Verification, 2),
            (Category::StandaloneReporting, 1),
            (Category::ReadingEditingAnalysis, 1)
        ]
    );
    assert_eq!(result.chained_reporting, 2);
    let shares: u64 = result
        .categories
        .iter()
        .map(|row| row.counts.requests)
        .sum();
    assert_eq!(shares, result.total.requests);
    let table = render_run(&result);
    assert!(table.contains("Chained reporting: 2 requests"), "{table}");
}

#[test]
fn requests_that_fit_several_categories_are_counted() {
    let world = World::new();
    world.session(
        "s",
        &[
            one(10, "m1", bash("git commit -m x && git push")),
            one(20, "m2", bash("cargo test")),
            one(30, "m3", bash("ls && cat notes.md")),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.several_categories, 1);
    assert!(render_run(&result).contains("more than one category: 1 request "));
}

#[test]
fn tool_output_is_summed_per_request_and_reported_per_category() {
    let world = World::new();
    world.session(
        "s",
        &[
            // One request with two calls; their results add up to 350 bytes.
            line(10, "m1", bash_call("c1", "cargo test"), (1, 1, 1, 1)),
            line(10, "m1", bash_call("c2", "cargo clippy"), (1, 1, 1, 1)),
            result(11, "c1", json!("a".repeat(100))),
            result(
                12,
                "c2",
                json!([{"type": "text", "text": "é".repeat(100)}, {"type": "text", "text": "SECRET"}]),
            ),
            // Another request in the same category with a 40 byte result.
            line(20, "m2", bash_call("c3", "cargo test"), (1, 1, 1, 1)),
            result(21, "c3", json!("b".repeat(40))),
            // A request whose result belongs to no known call is not counted.
            result(22, "unknown", json!("c".repeat(999))),
            one(30, "m3", text()),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));

    let verification = &result.categories[0];
    assert_eq!(verification.category, Category::Verification);
    assert_eq!(verification.counts.tool_output_bytes, Some(306 + 40));
    assert_eq!(verification.counts.largest_tool_output_bytes, Some(306));
    assert_eq!(result.total.tool_output_bytes, Some(346));
    let text_only = &result.categories[1];
    assert_eq!(text_only.counts.tool_output_bytes, Some(0));

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["categories"][0]["tool_output_bytes"], 346);
    let table = render_run(&result);
    assert!(!table.contains("SECRET"));
    let row = table.lines().find(|l| l.contains("verification")).unwrap();
    assert!(row.contains("346") && row.contains("306"), "{row}");
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

#[test]
fn a_session_without_subagent_logs_is_marked_coordinator_only() {
    let world = World::new();
    world.session("s", &[one(10, "m1", text())]);
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.subagents.requests, 0);
    assert_eq!(result.coordinator_only_sessions, ["s"]);
    assert!(
        result.notes.iter().any(|n| n.contains("coordinator-only")),
        "{:?}",
        result.notes
    );
}

#[test]
fn copilot_usage_records_without_a_message_stay_uncategorized_and_codex_ones_are_categorized() {
    let world = World::new();
    world.file(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-abc.jsonl",
        &[
            json!({"type": "session_meta", "timestamp": iso(0), "payload": {"id": "abc", "source": "cli"}}).to_string(),
            json!({"type": "turn_context", "timestamp": iso(0), "payload": {"model": "gpt-test", "turn_id": "turn-1"}}).to_string(),
            codex_usage(10, "r1"),
            codex_usage(20, "r2"),
        ],
    );
    world.file(
        "copilot/session-state/c1/events.jsonl",
        &[
            copilot_usage(30, "call1", false),
            copilot_usage(40, "call2", true),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["abc", "c1"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));

    assert_eq!(
        sources(&result),
        [
            (Category::TextOnly, false, 2),
            (Category::NotCategorized, false, 1),
            (Category::NotCategorized, true, 1)
        ]
    );
    // The Codex rows know their tool output (none here); the Copilot rows have no message to read it from.
    assert_eq!(result.categories[0].counts.tool_output_bytes, Some(0));
    let copilot = &result.categories[1].counts;
    assert_eq!(copilot.tokens.input, Some(40));
    assert_eq!(copilot.tool_output_bytes, None);
    let json = serde_json::to_value(&result).unwrap();
    assert!(json["categories"][1].get("tool_output_bytes").is_none());
    let table = render_run(&result);
    let line = table
        .lines()
        .find(|l| l.starts_with("  not categorized "))
        .expect(&table);
    assert!(line.contains("n/a"), "{line}");
    assert!(
        result
            .notes
            .iter()
            .any(|n| n.contains("no matching assistant message")),
        "{:?}",
        result.notes
    );
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
                    json!({"command": format!("factory run update {RUN_A} build running")}),
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

use crate::requests_pool::{Class, PoolOptions, pool, render_pool};

fn run_id(n: u64) -> String {
    format!("17938728000000000{n:02}")
}

/// A run with usage that is `tracked`, `partial` or missing, done or still running.
fn pooled_run(n: u64, done: bool, usage: &str) -> Run {
    let mut value = json!({
        "id": run_id(n), "title": format!("Run {n}"), "project": "/p", "stage": "finish",
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

#[test]
fn partial_open_excluded_and_empty_runs_are_left_out_with_their_reason() {
    let (world, mut runs) = pooled_world();
    runs.push(pooled_run(6, false, "tracked")); // open
    pooled_session(&world, 6, 4);
    runs.push(pooled_run(7, true, "partial")); // partial
    pooled_session(&world, 7, 4);
    runs.push(pooled_run(8, true, "tracked")); // to exclude
    pooled_session(&world, 8, 4);
    runs.push(pooled_run(9, true, "tracked")); // no log at all

    let options = PoolOptions {
        include_partial: false,
        exclude: vec![run_id(8), "999".to_string()],
    };
    let result = pooled(&world, &runs, &options);
    assert_eq!(result.included.len(), 5);
    let reason = |n: u64| {
        result
            .left_out
            .iter()
            .find(|l| l.run == run_id(n))
            .map(|l| l.reason.clone())
            .unwrap_or_default()
    };
    assert!(reason(6).contains("open"), "{}", reason(6));
    assert!(reason(7).contains("partial"), "{}", reason(7));
    assert!(reason(8).contains("--exclude"), "{}", reason(8));
    assert!(reason(9).contains("no requests"), "{}", reason(9));
    assert!(
        result.notes.iter().any(|n| n.contains("999")),
        "{:?}",
        result.notes
    );
    let table = render_pool(&result);
    assert!(
        table.contains("Left out") && table.contains(&run_id(7)),
        "{table}"
    );

    // With --include-partial the open and the partial run join.
    let all = PoolOptions {
        include_partial: true,
        exclude: vec![run_id(8)],
    };
    let result = pooled(&world, &runs, &all);
    assert_eq!(result.included.len(), 7);
    let classes: Vec<Class> = result.included.iter().map(|r| r.class).collect();
    assert!(classes.contains(&Class::Open) && classes.contains(&Class::Partial));
}

#[test]
fn the_pool_options_come_in_any_order_and_refuse_the_rest() {
    use crate::requests_pool::parse_options;
    let (options, json) = parse_options(&[
        "--exclude",
        "12",
        "--json",
        "--include-partial",
        "--exclude",
        "34",
    ])
    .unwrap();
    assert!(json && options.include_partial);
    assert_eq!(options.exclude, ["12", "34"]);
    let (options, json) = parse_options(&[]).unwrap();
    assert!(!json && !options.include_partial && options.exclude.is_empty());
    for bad in [
        &["--bogus"][..],
        &["--exclude"],
        &["--exclude", "x1"],
        &["--json", "--json"],
    ] {
        assert!(parse_options(bad).is_err(), "{bad:?}");
    }
}

// --- which run a command updates, and which subagent logs count ---------------------------------------

#[test]
fn only_a_factory_command_claims_a_run_not_a_command_that_names_one() {
    let world = World::new();
    world.session(
        "named",
        &[
            one(10, "n1", bash(&format!("grep -n \"factory run update {RUN_A}\" README.md"))),
            one(20, "n2", bash(&format!("echo \"factory run update {RUN_A} build running\""))),
            one(
                30,
                "n3",
                bash(&format!("gh pr create --body-file - <<'EOF'\nRun factory run update {RUN_A} to report\nEOF")),
            ),
        ],
    );
    world.session(
        "real",
        &[one(
            10,
            "r1",
            bash(&format!(
                "cd /repo && factory run update {RUN_A} build running --activity x"
            )),
        )],
    );
    let target = run(RUN_A, 0, Some(100), &[]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.sessions, ["real"]);
}

#[test]
fn a_session_with_subagent_logs_is_not_coordinator_only_even_when_they_fall_outside_the_run() {
    let world = World::new();
    world.session("s", &[one(10, "m1", text())]);
    world.file(
        "claude/projects/slug/s/subagents/agent-1.jsonl",
        &[sidechain(one(900, "late", text()))],
    );
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.total.requests, 1);
    assert!(
        result.coordinator_only_sessions.is_empty(),
        "{:?}",
        result.notes
    );
}

#[test]
fn a_command_of_thousands_of_prefix_words_is_still_classified() {
    let command = format!("{}cargo test", "sudo ".repeat(12_000));
    // Only the start of a command can name the program, so a hostile one costs no more than a normal one.
    assert_eq!(category_of(&command), Category::ReadingEditingAnalysis);
    let command = format!("git {}commit -m x", "-C x ".repeat(12_000));
    assert_eq!(category_of(&command), Category::ReadingEditingAnalysis);
}

#[cfg(unix)]
#[test]
fn a_linked_subagent_folder_is_not_followed() {
    let world = World::new();
    world.session("s", &[one(10, "m1", text())]);
    world.file(
        "elsewhere/agent-1.jsonl",
        &[sidechain(one(20, "x1", text()))],
    );
    let session_dir = world.dir.path().join("claude/projects/slug/s");
    fs::create_dir_all(&session_dir).unwrap();
    std::os::unix::fs::symlink(
        world.dir.path().join("elsewhere"),
        session_dir.join("subagents"),
    )
    .unwrap();
    let target = run(RUN_A, 0, Some(100), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.total.requests, 1);
    assert_eq!(result.subagents.requests, 0);
}

#[test]
fn the_pooled_view_keeps_the_notes_that_say_a_reading_was_cut_short() {
    let (world, runs) = pooled_world();
    // A deadline that has already passed: every log read stops at once.
    let result = pool(
        &runs,
        &world.roots(),
        T0 + 10_000_000,
        Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("the clock is far enough from its start"),
        &PoolOptions::default(),
    );
    assert!(
        result.notes.iter().any(|n| n.contains("took too long")),
        "{:?}",
        result.notes
    );
}

// --- quotes and wrapper words around an update command -----------------------------------------------

/// The sessions that claim `RUN_A` among the given commands, one session per command.
fn claimed_by(commands: &[String]) -> Vec<String> {
    let world = World::new();
    for (index, command) in commands.iter().enumerate() {
        world.session(
            &format!("s{index}"),
            &[one(10, &format!("m{index}"), bash(command))],
        );
    }
    let target = run(RUN_A, 0, Some(100), &[]);
    let mut sessions = world
        .analyze(&target, std::slice::from_ref(&target))
        .sessions;
    sessions.sort();
    sessions
}

#[test]
fn text_inside_quotes_never_claims_a_run() {
    let id = RUN_A;
    assert!(
        claimed_by(&[
            format!("echo \"done; factory run update {id} ok\""),
            format!("git commit -m \"wip && factory run update {id}\""),
            format!("gh pr create --body \"Summary\nfactory run update {id} check done\nmore\""),
            format!("echo 'a | factory run update {id}'"),
            format!("echo \"say \\\"x\\\"; factory run update {id}\""),
        ])
        .is_empty()
    );
}

#[test]
fn a_real_update_command_claims_however_it_is_wrapped() {
    let id = RUN_A;
    let commands: Vec<String> = [
        "factory run update ID build running",
        "/usr/local/bin/factory run update ID build running",
        "cd /repo && factory run update ID build running",
        "(factory run update ID build running)",
        "if factory run update ID build running; then echo ok; fi",
        "env X=1 factory run update ID build running",
        "timeout 60 factory run update ID build running",
        "X=1 sudo factory run update \"ID\" build running",
        "ls | factory run update ID build running",
    ]
    .iter()
    .map(|c| c.replace("ID", id))
    .collect();
    for command in &commands {
        assert_eq!(
            claimed_by(std::slice::from_ref(command)).len(),
            1,
            "{command}"
        );
    }
}

#[test]
fn words_inside_quotes_do_not_decide_the_category() {
    assert_eq!(
        category_of("echo \"a; git push\""),
        Category::ReadingEditingAnalysis
    );
    assert_eq!(
        category_of("git commit -m \"fix; cargo test\" && ls"),
        Category::Commit
    );
}

// --- comments, here-strings and wrappers with options ------------------------------------------------

#[test]
fn an_update_claims_after_comments_here_strings_and_wrappers_with_options() {
    let commands: Vec<String> = [
        "cargo test # don't skip\nfactory run update ID b",
        "cat <<< \"x\"\nfactory run update ID b",
        "cat <<'EOF';\nhi\nEOF\nfactory run update ID b",
        "echo $((1<<2))\nfactory run update ID b",
        "(cd x && factory run update ID)",
        "{ factory run update ID; }",
        "timeout -s KILL 60 factory run update ID b",
        "timeout -k 5 60 factory run update ID b",
        "env -i factory run update ID b",
        "env -u HOME X=1 factory run update ID b",
    ]
    .iter()
    .map(|c| c.replace("ID", RUN_A))
    .collect();
    for command in &commands {
        assert_eq!(
            claimed_by(std::slice::from_ref(command)).len(),
            1,
            "{command}"
        );
    }
}

#[test]
fn text_that_is_not_a_command_never_claims_a_run() {
    assert!(
        claimed_by(&[
            format!("cat <<\\EOF\nfactory run update {RUN_A} b\nEOF"),
            format!("cat <<\"EOF\"\nfactory run update {RUN_A} b\nEOF"),
            format!("ls # factory run update {RUN_A}"),
            format!("# factory run update {RUN_A}\nls"),
        ])
        .is_empty()
    );
    // `#` inside a word is not a comment.
    assert_eq!(
        category_of("echo a#b; git push"),
        Category::PrCreationAndPush
    );
}

// --- commands as agents write them -------------------------------------------------------------------

#[test]
fn a_report_behind_a_variable_and_a_directory_change_is_standalone_reporting() {
    for command in [
        "R=$(cat /tmp/run-id) && cd /repo/wt && factory run update $R build running --activity x",
        "R=`cat /tmp/run-id`; factory run update \"$R\" build running",
        "S=/tmp/scratch R=$(cat $S/id) && factory run update ${R} build running",
    ] {
        assert_eq!(
            category_of(command),
            Category::StandaloneReporting,
            "{command}"
        );
    }
}

#[test]
fn a_substitution_is_one_word_and_the_commands_inside_it_count() {
    assert_eq!(
        category_of("gh pr create --title \"$(git log -1 --format=%s)\" --body x"),
        Category::PrCreationAndPush
    );
    assert_eq!(
        category_of("echo \"$(git push origin x)\""),
        Category::PrCreationAndPush
    );
    assert_eq!(
        category_of("echo `git push origin x`"),
        Category::PrCreationAndPush
    );
    assert_eq!(
        category_of("OUT=$(cargo test 2>&1) && echo done"),
        Category::Verification
    );
    // Separators inside a substitution do not split the command around it.
    assert_eq!(
        category_of("git commit -m \"$(date; whoami)\" && ls"),
        Category::Commit
    );
    // Arithmetic is not a command substitution.
    assert_eq!(
        category_of("echo $((1+2)) && git push"),
        Category::PrCreationAndPush
    );
    // A substitution inside single quotes is text.
    assert_eq!(
        category_of("echo '$(git push)'"),
        Category::ReadingEditingAnalysis
    );
}

#[test]
fn a_literal_assignment_in_the_same_command_resolves_the_variable_that_names_the_run() {
    let id = RUN_A;
    let claiming: Vec<String> = [
        "R=ID && factory run update $R build running",
        "R=ID; cd /repo && factory run update \"$R\" build running",
        "R='ID'\nfactory run update ${R} build running",
    ]
    .iter()
    .map(|c| c.replace("ID", id))
    .collect();
    for command in &claiming {
        assert_eq!(
            claimed_by(std::slice::from_ref(command)).len(),
            1,
            "{command}"
        );
    }
    // A value from a file or another command cannot be resolved, so nothing is claimed.
    for command in [
        "R=$(cat /tmp/run-id) && factory run update $R build running".to_string(),
        "factory run update $R build running".to_string(),
        format!("R=other && factory run update $R build running # {id}"),
    ] {
        assert!(
            claimed_by(std::slice::from_ref(&command)).is_empty(),
            "{command}"
        );
    }
}

#[test]
fn thousands_of_open_substitutions_do_not_overflow_the_stack() {
    let command = "$(".repeat(20_000) + "git push";
    // The answer is not the point: the call ends, with no overflow of the stack.
    let _ = category_of(&command);
    let command = "`".repeat(20_000) + "git push";
    let _ = category_of(&command);
}

#[test]
fn a_filter_after_a_pipe_is_plumbing_for_the_command_before_it() {
    assert_eq!(
        category_of("factory run update 1 build running --activity x 2>&1 | tail -1"),
        Category::StandaloneReporting
    );
    assert_eq!(
        category_of("cargo test 2>&1 | tail -5"),
        Category::Verification
    );
    assert_eq!(
        category_of("ls | grep src | wc -l"),
        Category::ReadingEditingAnalysis
    );
    // A command that follows `&&` after a pipeline is work of its own again.
    assert_eq!(
        category_of("factory run update 1 build running | tail -1 && ls"),
        Category::ReadingEditingAnalysis
    );
}

#[test]
fn the_request_that_starts_a_run_counts_even_though_it_precedes_the_runs_first_mark() {
    let world = World::new();
    world.session(
        "s",
        &[
            // 30 seconds before the run's first mark: it runs `factory run start` and makes the worktree.
            one(370, "start", bash("git fetch -q origin develop && factory run start \"A title\" && git worktree add -b x ../y")),
            // Also before the first mark, but not workspace work: not the run's.
            one(375, "before", text()),
            // Workspace work minutes before the first mark: another piece of work.
            one(100, "far", bash("git worktree add -b z ../w")),
            one(420, "inside", text()),
        ],
    );
    let target = run(RUN_A, 400, Some(900), &["s"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(
        rows(&result),
        [(Category::Workspace, 1), (Category::TextOnly, 1)]
    );
}

#[test]
fn a_here_document_inside_a_substitution_does_not_swallow_the_rest_of_the_line() {
    let id = RUN_A;
    // The usual Claude Code commit: the message sits in a here-document, with an apostrophe.
    let commit = format!(
        "git commit -m \"$(cat <<'EOF'\nDon't (stop\nEOF\n)\" && factory run update {id} build running"
    );
    assert_eq!(
        claimed_by(std::slice::from_ref(&commit)).len(),
        1,
        "{commit}"
    );
    assert_eq!(
        category_of("echo \"$(cat <<'EOF'\nDon't\nEOF\n)\" && git push"),
        Category::PrCreationAndPush
    );
}

#[test]
fn only_read_only_filters_and_helpers_are_plumbing() {
    for (command, expected) in [
        (
            "H=$(git rev-parse --short HEAD) && factory run update 1 build running",
            Category::StandaloneReporting,
        ),
        (
            "factory run update 1 build running 2>&1 | grep -v x | head -3",
            Category::StandaloneReporting,
        ),
        (
            "D=$(date +%s) && factory run update 1 build running",
            Category::StandaloneReporting,
        ),
        // Work that happens in a pipe or a substitution is still work.
        (
            "echo x | git apply && factory run update 1 build running",
            Category::ReadingEditingAnalysis,
        ),
        (
            "R=$(python3 make_edits.py) && factory run update 1 build running",
            Category::ReadingEditingAnalysis,
        ),
        (
            "factory run update 1 build running | tee out.txt",
            Category::ReadingEditingAnalysis,
        ),
        (
            "ls | xargs sed -i s/a/b/ && factory run update 1 build running",
            Category::ReadingEditingAnalysis,
        ),
    ] {
        assert_eq!(category_of(command), expected, "{command}");
    }
}

#[test]
fn a_run_start_request_near_two_overlapping_runs_is_in_neither() {
    let world = World::new();
    // Inside run A, and 30 seconds before run B's first mark.
    world.session(
        "s",
        &[one(420, "start", bash("git worktree add -b x ../y"))],
    );
    let a = run(RUN_A, 400, Some(900), &["s"]);
    let b = run(RUN_B, 450, Some(900), &["s"]);
    let runs = [a.clone(), b];
    let first = world.analyze(&a, &runs);
    assert_eq!(first.total.requests, 0);
    assert_eq!(first.ambiguous.requests, 1);
}

#[test]
fn read_only_git_and_gh_forms_are_helpers_too() {
    for command in [
        "H=$(git -C ../wt rev-parse HEAD) && factory run update 1 build running",
        "B=$(git branch --show-current) && factory run update 1 build running",
        "S=$(git status --short | head -3) && factory run update 1 build running",
        "factory run update 1 finish done --summary finish=Shipped --activity \"Opened $(gh pr view --json url -q .url)\"",
        "L=$(git log -1 --format=%s) && factory run update 1 build running",
    ] {
        assert_eq!(
            category_of(command),
            Category::StandaloneReporting,
            "{command}"
        );
    }
    // Forms that change something are not helpers.
    for command in [
        "B=$(git branch -D old) && factory run update 1 build running",
        "U=$(gh pr merge 5) && factory run update 1 build running",
        "X=$(git diff --output=/tmp/p) && factory run update 1 build running",
        "X=$(git -C d log --output=/tmp/p) && factory run update 1 build running",
    ] {
        assert_ne!(
            category_of(command),
            Category::StandaloneReporting,
            "{command}"
        );
    }
}

#[test]
fn a_here_string_inside_a_substitution_is_not_a_here_document() {
    assert_eq!(
        category_of("X=$(tr a b <<<abc\ncat)\nfactory run update 1 build running"),
        Category::StandaloneReporting
    );
}

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

fn codex_output(kind: &str, call: &str, output: Value) -> String {
    json!({"type": "response_item", "timestamp": iso(6), "payload": {"type": kind, "call_id": call, "output": output}}).to_string()
}

/// The category of a lone Codex response whose snippet is `snippet`.
fn codex_category_of(snippet: &str) -> Category {
    let world = World::new();
    world.file(
        CODEX_FILE,
        &[
            codex_meta("abc", None),
            codex_exec("c1", snippet),
            codex_usage(10, "r1"),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["abc"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.categories.len(), 1, "{snippet}: {result:?}");
    result.categories[0].category
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
fn a_codex_command_is_read_from_its_javascript_string_literal() {
    for (snippet, expected) in [
        (
            r#"text(await tools.exec_command({cmd:"git push origin x"}))"#,
            Category::PrCreationAndPush,
        ),
        (
            r#"text(await tools.exec_command({cmd:'git push origin x',workdir:"/w"}))"#,
            Category::PrCreationAndPush,
        ),
        (
            "text(await tools.exec_command({workdir:\"/w\", cmd: `git push`}))",
            Category::PrCreationAndPush,
        ),
        (
            r#"await tools.exec_command({ "cmd" : "cargo test\n" })"#,
            Category::Verification,
        ),
        // Escapes are decoded: the quote and the newline do not end the command early.
        (
            r#"await tools.exec_command({cmd:"echo \"a; git push\" && cargo test\ngit commit -m x"})"#,
            Category::Commit,
        ),
        (
            r#"await tools.exec_command({cmd:"git \u0070ush"})"#,
            Category::PrCreationAndPush,
        ),
        // Not a plain literal: it cannot be read, so it is a tool call that is not a command.
        (
            "await tools.exec_command({cmd})",
            Category::ReadingEditingAnalysis,
        ),
        (
            "await tools.exec_command({cmd: command})",
            Category::ReadingEditingAnalysis,
        ),
        (
            r#"await tools.exec_command({cmd:"git " + "push"})"#,
            Category::ReadingEditingAnalysis,
        ),
        (
            "await tools.exec_command({cmd:`git ${what}`})",
            Category::ReadingEditingAnalysis,
        ),
        // Other tools, and a script that runs no command.
        (
            r#"await tools.write_stdin({session_id: 1, chars: "git push"})"#,
            Category::ReadingEditingAnalysis,
        ),
        (
            r#"await tools.web__run({open:[{ref_id:"https://example.com"}]})"#,
            Category::ReadingEditingAnalysis,
        ),
        ("text(String(1+1))", Category::ReadingEditingAnalysis),
        // A string that is never closed does not run away.
        (
            r#"await tools.exec_command({cmd:"git push"#,
            Category::ReadingEditingAnalysis,
        ),
    ] {
        assert_eq!(codex_category_of(snippet), expected, "{snippet}");
    }
}

#[test]
fn codex_tool_output_is_counted_on_the_request_that_made_the_call_even_when_it_arrives_late() {
    let world = World::new();
    world.file(
        CODEX_FILE,
        &[
            codex_meta("abc", None),
            codex_exec_commands("c1", &["cargo test"]),
            codex_usage(10, "r1"),
            codex_exec_commands("c2", &["cargo clippy"]),
            // The result of the first call arrives after the second response's call.
            codex_output("custom_tool_call_output", "c1", json!([{"type": "input_text", "text": "a".repeat(100)}])),
            codex_usage(20, "r2"),
            codex_output(
                "custom_tool_call_output",
                "c2",
                json!([{"type": "input_text", "text": "é".repeat(15)}, {"type": "input_text", "text": "SECRET"}]),
            ),
            codex_function_call("c3", "wait_agent"),
            codex_usage(30, "r3"),
            codex_output("function_call_output", "c3", json!("b".repeat(7))),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["abc"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    let verification = &result.categories[0];
    assert_eq!(verification.category, Category::Verification);
    assert_eq!(verification.counts.tool_output_bytes, Some(100 + 36));
    assert_eq!(verification.counts.largest_tool_output_bytes, Some(100));
    let other = &result.categories[1];
    assert_eq!(other.category, Category::ReadingEditingAnalysis);
    assert_eq!(other.counts.tool_output_bytes, Some(7));
    assert!(!serde_json::to_string(&result).unwrap().contains("SECRET"));
    assert!(!render_run(&result).contains("SECRET"));
}

#[test]
fn a_codex_session_claims_a_run_only_through_an_executed_update_or_a_recorded_id() {
    let world = World::new();
    let update = format!("factory run update {RUN_A} build running --activity x");
    world.file(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-executed.jsonl",
        &[
            codex_meta("executed", None),
            codex_exec_commands("e1", &[&format!("cd /w && {update}")]),
            codex_usage(10, "e-r1"),
        ],
    );
    world.file(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-mentioned.jsonl",
        &[
            codex_meta("mentioned", None),
            json!({"type": "response_item", "timestamp": iso(2), "payload": {"type": "message", "role": "user",
                "content": [{"type": "input_text", "text": update.clone()}]}}).to_string(),
            codex_exec_commands("m1", &[&format!("grep -n \"{update}\" README.md"), &format!("echo '{update}'")]),
            codex_output("custom_tool_call_output", "m1", json!([{"type": "input_text", "text": update.clone()}])),
            codex_usage(10, "m-r1"),
        ],
    );
    world.file(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-00-recorded.jsonl",
        &[codex_meta("recorded", None), codex_usage(10, "x-r1")],
    );
    let target = run(RUN_A, 0, Some(100), &["recorded"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    let mut sessions = result.sessions.clone();
    sessions.sort();
    assert_eq!(sessions, ["executed", "recorded"]);
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

#[test]
fn a_codex_response_that_is_recorded_twice_counts_once_with_its_largest_counts() {
    let world = World::new();
    let mut growing = codex_usage(10, "r1");
    growing = growing.replace("\"output_tokens\":20", "\"output_tokens\":99");
    world.file(
        CODEX_FILE,
        &[codex_meta("abc", None), codex_usage(10, "r1"), growing],
    );
    let target = run(RUN_A, 0, Some(100), &["abc"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(result.total.requests, 1);
    assert_eq!(result.total.tokens.output, Some(99));
}

fn codex_task_started() -> String {
    json!({"type": "event_msg", "timestamp": iso(0), "payload": {"type": "task_started", "turn_id": "t"}}).to_string()
}

#[test]
fn the_history_a_codex_subagent_inherits_is_not_its_own_work() {
    let world = World::new();
    world.file(
        CODEX_FILE,
        &[
            codex_meta("abc", None),
            codex_exec_commands("a1", &["cargo test"]),
            codex_usage(10, "r1"),
        ],
    );
    // A subagent rollout opens with a copy of the parent's history: calls and results, no token records.
    world.file(
        "codex/sessions/2026/11/05/rollout-2026-11-05T10-00-30-sub1.jsonl",
        &[
            codex_meta("sub1", Some("abc")),
            codex_task_started(),
            codex_exec_commands("old1", &["git push origin x"]),
            codex_output(
                "custom_tool_call_output",
                "old1",
                json!([{"type": "input_text", "text": "x".repeat(500)}]),
            ),
            // The subagent's own turn starts here.
            codex_task_started(),
            codex_exec_commands("s1", &["git log -1"]),
            codex_usage(20, "s-r1"),
            codex_output(
                "custom_tool_call_output",
                "s1",
                json!([{"type": "input_text", "text": "y".repeat(10)}]),
            ),
        ],
    );
    let target = run(RUN_A, 0, Some(100), &["abc"]);
    let result = world.analyze(&target, std::slice::from_ref(&target));
    assert_eq!(
        sources(&result),
        [
            (Category::Verification, false, 1),
            (Category::ReadingEditingAnalysis, true, 1)
        ]
    );
    let subagent = &result.categories[1].counts;
    assert_eq!(subagent.tool_output_bytes, Some(10));
}

#[test]
fn a_hostile_codex_snippet_is_read_in_one_pass() {
    // None of these may end in a panic or take longer than their length allows; the answer is a tool call
    // that is not a command.
    let calls_open = "tools.exec_command({a:(".repeat(3_000);
    let calls_template = "tools.exec_command({a:`".repeat(3_000);
    let unicode = "tools.exec_command({cmd:\"\\u{".repeat(3_000);
    let brackets = "tools.exec_command({a:[)".repeat(10_000);
    let braces = "tools.exec_command({a:{)".repeat(10_000);
    let long_literal = format!(
        "await tools.exec_command({{cmd:\"{}\"}})",
        "a".repeat(200_000)
    );
    for snippet in [calls_open, calls_template, unicode, brackets, braces] {
        assert_eq!(
            codex_category_of(&snippet),
            Category::ReadingEditingAnalysis
        );
    }
    // A very long command is cut, and still classified by its start.
    assert_eq!(
        codex_category_of(&long_literal),
        Category::ReadingEditingAnalysis
    );
}

#[test]
fn a_call_after_a_broken_one_in_the_same_snippet_is_still_read() {
    assert_eq!(
        codex_category_of(
            "await Promise.allSettled([tools.exec_command({cmd}), tools.exec_command({a:[)}), tools.exec_command({cmd:\"git push\"})])"
        ),
        Category::PrCreationAndPush
    );
}

#[test]
fn javascript_comments_inside_a_call_do_not_hide_it_or_the_calls_after_it() {
    for (snippet, expected) in [
        // A quote in a comment must not open a string that runs to the end.
        (
            "tools.exec_command({cmd:\"ls\", // don't wait\n yield_time_ms: 1000}); await tools.exec_command({cmd:\"git push\"})",
            Category::PrCreationAndPush,
        ),
        (
            "tools.exec_command({cmd:\"ls\", /* it's slow */ yield_time_ms: 1000}); await tools.exec_command({cmd:\"git push\"})",
            Category::PrCreationAndPush,
        ),
        // A comment before `cmd` or between properties.
        (
            "await tools.exec_command({ // don't forget the dir\n workdir: \"/w\", /* the command: */ cmd: \"git push\" })",
            Category::PrCreationAndPush,
        ),
        (
            "await tools.exec_command({cmd: \"git push\" // done\n })",
            Category::PrCreationAndPush,
        ),
    ] {
        assert_eq!(codex_category_of(snippet), expected, "{snippet}");
    }
}

#[test]
fn absorbing_a_root_merges_notes_once_ors_the_flags_and_says_when_to_stop() {
    let note = |text: &str| text.to_string();
    let mut all = RequestRead {
        notes: vec![note("a")],
        root_found: false,
        ..RequestRead::default()
    };
    let first = RequestRead {
        notes: vec![note("a"), note("b")],
        root_found: true,
        ..RequestRead::default()
    };
    assert!(!all.absorb(first));
    assert_eq!(all.notes, ["a", "b"]);
    assert!(all.root_found && !all.timed_out && !all.truncated);
    let second = RequestRead {
        truncated: true,
        ..RequestRead::default()
    };
    assert!(all.absorb(second));
    assert!(all.root_found && all.truncated);
}
