//! Reads the model requests of Copilot `events.jsonl` files for `jidoka run requests`.
//!
//! Like the Claude Code and Codex readers, it keeps of each request only the time, the token counts, a
//! category taken from its commands and the size of its tool output. A request is a `session.usage_record`;
//! the `assistant.message` with the same `apiCallId` holds its tool calls, and each call's
//! `tool.execution_complete` holds its result. Subagents write into the same file, and their requests carry
//! the `sub-agent` initiator. The command text, tool output and paths are dropped as soon as they are
//! classified: nothing but categories, flags and counts leaves this module.

use crate::{
    request_logs::{
        Category, Request, RequestRead, SessionRequests, Variables, classify, result_bytes,
        run_claims, truncate,
    },
    usage::Tokens,
    usage_logs::{
        LogFile, MTIME_SLACK_MS, ReadLimits, copilot_tokens, mentioned_runs, merge_largest,
        model_name, modified_ms, read_text, sub_dirs, timestamp,
    },
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    time::Instant,
};

/// The tool calls of one `assistant.message`.
#[derive(Default)]
struct Calls {
    commands: Vec<String>,
    other_tool: bool,
    ids: Vec<String>,
}

struct Usage {
    ts: u64,
    tokens: Tokens,
    subagent: bool,
}

/// A session's id, the runs it claims and its requests with their keys.
type ParsedSession = (String, HashSet<String>, Vec<(String, Request)>);

/// The run ids in `ids` that a Copilot log names: those written right after a run command, and those a
/// bash command reaches through a shell variable set to a literal (`id=<id> && jidoka run update $id`),
/// with variables carried across the commands in order.
pub(crate) fn copilot_claims(text: &str, ids: &HashSet<&str>) -> HashSet<String> {
    let mut claims = mentioned_runs(text, ids);
    let mut variables = Variables::new();
    for line in text
        .lines()
        .filter(|line| line.contains("\"assistant.message\""))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for request in data["data"]["toolRequests"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (true, Some(command)) = (
                request["name"] == "bash",
                request["arguments"]["command"].as_str(),
            ) {
                claims.extend(run_claims(truncate(command), ids, &mut variables));
            }
        }
    }
    claims
}

/// The requests of one log, keyed by `apiCallId` (or by line when a record has none).
fn parse_events(text: &str, session: &str) -> (BTreeMap<String, Request>, usize) {
    let mut malformed = 0;
    let mut calls: HashMap<String, Calls> = HashMap::new();
    let mut results: HashMap<String, u64> = HashMap::new();
    let mut usages: Vec<(String, Option<String>, Usage)> = Vec::new();
    for (number, line) in text.lines().enumerate().filter(|(_, line)| {
        line.contains("\"session.usage_record\"")
            || line.contains("\"assistant.message\"")
            || line.contains("\"tool.execution_complete\"")
    }) {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            malformed += 1;
            continue;
        };
        let body = &data["data"];
        match data["type"].as_str() {
            Some("assistant.message") => {
                let Some(call) = body["apiCallId"].as_str() else {
                    continue;
                };
                let entry: &mut Calls = calls.entry(call.to_string()).or_default();
                for request in body["toolRequests"].as_array().into_iter().flatten() {
                    if let Some(id) = request["toolCallId"].as_str() {
                        entry.ids.push(id.to_string());
                    }
                    match (request["name"] == "bash")
                        .then(|| request["arguments"]["command"].as_str())
                        .flatten()
                    {
                        Some(command) => entry.commands.push(truncate(command).to_string()),
                        None => entry.other_tool = true,
                    }
                }
            }
            Some("tool.execution_complete") => {
                if let (Some(id), content) =
                    (body["toolCallId"].as_str(), &body["result"]["content"])
                    && !content.is_null()
                {
                    results.insert(id.to_string(), result_bytes(content));
                }
            }
            Some("session.usage_record") => {
                let usage = &body["usage"];
                let (Some(_), Some(ts), Some(tokens)) = (
                    model_name(&usage["model"]),
                    timestamp(data.get("timestamp")),
                    copilot_tokens(usage),
                ) else {
                    continue;
                };
                let call = usage["apiCallId"].as_str().map(str::to_string);
                let key = call
                    .clone()
                    .unwrap_or_else(|| format!("copilot|{session}|line {number}"));
                usages.push((
                    key,
                    call,
                    Usage {
                        ts,
                        tokens,
                        subagent: usage["initiator"] == "sub-agent",
                    },
                ));
            }
            _ => {}
        }
    }
    let mut requests: BTreeMap<String, Request> = BTreeMap::new();
    for (key, call, usage) in usages {
        // A record with no message is not known to have made no tool call, so it stays uncategorized.
        let message = call.and_then(|call| calls.get(&call));
        let (category, chained_reporting, several_categories, tool_output_bytes) = match message {
            Some(message) => {
                let class = classify(
                    &message.commands,
                    message.other_tool,
                    !message.commands.is_empty() || message.other_tool,
                );
                let sizes: Option<Vec<u64>> = message
                    .ids
                    .iter()
                    .map(|id| results.get(id).copied())
                    .collect();
                (
                    class.category,
                    class.chained_reporting,
                    class.several_categories,
                    sizes.map(|sizes| sizes.iter().fold(0u64, |a, b| a.saturating_add(*b))),
                )
            }
            None => (Category::NotCategorized, false, false, None),
        };
        let request = Request {
            ts: usage.ts,
            category,
            tokens: usage.tokens,
            chained_reporting,
            several_categories,
            tool_output_bytes,
            subagent: usage.subagent,
        };
        match requests.get_mut(&key) {
            Some(existing) => merge_largest(&mut existing.tokens, &request.tokens),
            None => {
                requests.insert(key, request);
            }
        }
    }
    (requests, malformed)
}

/// Reads the Copilot sessions under `root`: every session that names a run in `ids` in a
/// `jidoka run update`, `run inspect`, `run stats` or `run check` command or a `jidoka pr open`,
/// `pr sync` or `pr wait` command, or was recorded for one in `recorded`.
pub(crate) fn read_copilot_requests(
    root: &Path,
    ids: &HashSet<&str>,
    recorded: &HashSet<&str>,
    from: u64,
    deadline: Instant,
    limits: ReadLimits,
) -> RequestRead {
    let mut out = RequestRead {
        root_found: root.is_dir(),
        ..RequestRead::default()
    };
    if !out.root_found {
        return out;
    }
    let (mut files, mut bytes) = (0usize, 0u64);
    let mut malformed = 0;
    let mut sessions: Vec<ParsedSession> = Vec::new();
    for dir in sub_dirs(root) {
        let session = dir.file_name().to_string_lossy().into_owned();
        let path = dir.path().join("events.jsonl");
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        if Instant::now() > deadline {
            out.timed_out = true;
        }
        files += 1;
        if files > limits.files || bytes > limits.bytes {
            out.truncated = true;
        }
        if out.timed_out || out.truncated {
            break;
        }
        if modified_ms(&path).is_some_and(|m| m.saturating_add(MTIME_SLACK_MS) < from) {
            continue;
        }
        let is_recorded = recorded.contains(session.as_str());
        let text = match read_text(&path) {
            Ok(LogFile::Content(text)) => text,
            Ok(LogFile::Skipped) => continue,
            Ok(LogFile::TooBig) => {
                if is_recorded {
                    out.notes
                        .push("a session log larger than 64 MiB was not read".into());
                }
                continue;
            }
            Err(error) => {
                if is_recorded {
                    out.notes
                        .push(format!("a session log could not be read: {error}"));
                }
                continue;
            }
        };
        bytes += text.len() as u64;
        let claims = copilot_claims(&text, ids);
        if claims.is_empty() && !is_recorded {
            continue;
        }
        let (requests, bad) = parse_events(&text, &session);
        malformed += bad;
        sessions.push((session, claims, requests.into_iter().collect()));
    }
    if malformed > 0 {
        out.notes.push(format!(
            "{malformed} malformed Copilot log line(s) were skipped"
        ));
    }
    // A response that two logs hold (a resumed session copies its history) counts once.
    let mut seen: HashSet<String> = HashSet::new();
    for (id, claims, requests) in sessions {
        let mut requests: Vec<Request> = requests
            .into_iter()
            .filter(|(key, _)| seen.insert(key.clone()))
            .map(|(_, request)| request)
            .collect();
        requests.sort_by_key(|request| request.ts);
        out.sessions.push(SessionRequests {
            id,
            has_subagent_logs: requests.iter().any(|request| request.subagent),
            claims,
            requests,
        });
    }
    out
}
