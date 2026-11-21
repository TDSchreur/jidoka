//! The Copilot CLI adapter: `session-state/<session>/events.jsonl`. Subagents write into the same file, and
//! their usage records carry the `sub-agent` initiator.

use super::*;

/// The tool calls of one `assistant.message`.
#[derive(Default)]
struct Calls {
    commands: Vec<String>,
    other_tool: bool,
    ids: Vec<String>,
}

/// The tokens of a Copilot `usage` object, or `None` when it reports neither input nor output. Copilot
/// counts cache reads and writes inside `inputTokens`; both are taken out of input. Reasoning tokens are part
/// of output.
fn copilot_tokens(usage: &Value) -> Option<Tokens> {
    let (input, read, write, output) = (
        count(usage.get("inputTokens")),
        count(usage.get("cacheReadTokens")),
        count(usage.get("cacheWriteTokens")),
        count(usage.get("outputTokens")),
    );
    if input.is_none() && output.is_none() {
        return None;
    }
    Some(Tokens {
        input: input.map(|n| {
            n.saturating_sub(read.unwrap_or(0))
                .saturating_sub(write.unwrap_or(0))
        }),
        cache_read: read,
        cache_write: write,
        output,
        reasoning: count(usage.get("reasoningTokens")),
    })
}

/// The skills a Copilot log shows used: `skill` tool calls.
fn copilot_skills(text: &str) -> Vec<(u64, String)> {
    let mut skills = Vec::new();
    for line in text
        .lines()
        .filter(|line| line.contains("\"tool.execution_start\""))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if data["type"] != "tool.execution_start" || data["data"]["toolName"] != "skill" {
            continue;
        }
        if let (Some(ts), Some(name)) = (
            timestamp(data.get("timestamp")),
            data["data"]["arguments"]["skill"]
                .as_str()
                .and_then(skill_name),
        ) {
            skills.push((ts, name));
        }
    }
    skills
}

/// What one `events.jsonl` gave.
#[derive(Default)]
struct CopilotLog {
    /// The responses and their keys.
    responses: Vec<(String, Response)>,
    /// The run ids an executed Jidoka command in this log names. A bash command can reach the id through a
    /// shell variable set to a literal (`id=<id> && jidoka run update $id`), with variables carried across
    /// the commands in order.
    claims: HashSet<String>,
    skipped: Skipped,
}

/// One `session.usage_record`, before it is joined to its message.
struct Record {
    key: String,
    call: Option<String>,
    ts: u64,
    model: String,
    effort: Option<String>,
    tokens: Tokens,
    subagent: bool,
}

/// Gives each usage record the tool calls of its message and the size of their results.
fn join_calls(
    records: Vec<Record>,
    calls: &HashMap<String, Calls>,
    results: &HashMap<String, u64>,
    session: &str,
) -> Vec<(String, Response)> {
    let mut responses = Vec::new();
    for record in records {
        // A record with no message is not known to have made no tool call.
        let message = record.call.as_ref().and_then(|call| calls.get(call));
        let sizes: Option<Vec<u64>> = message.and_then(|message| {
            message
                .ids
                .iter()
                .map(|id| results.get(id).copied())
                .collect()
        });
        responses.push((
            record.key,
            Response {
                session: session.to_string(),
                ts: record.ts,
                model: record.model,
                effort: record.effort,
                tokens: record.tokens,
                subagent: record.subagent,
                commands: message.map(|m| m.commands.clone()).unwrap_or_default(),
                other_tool: message.is_some_and(|m| m.other_tool),
                any_tool: message.is_some_and(|m| !m.commands.is_empty() || m.other_tool),
                calls_known: message.is_some(),
                tool_output_bytes: sizes
                    .map(|sizes| sizes.iter().fold(0u64, |a, b| a.saturating_add(*b))),
            },
        ));
    }
    responses
}

/// The responses and claims of one log. A response is a `session.usage_record`; the `assistant.message`
/// with the same `apiCallId` holds its tool calls, and each call's `tool.execution_complete` holds its
/// result. Each record also names the effort of its call as `reasoningEffort`.
fn parse_copilot(text: &str, session: &str, ids: &HashSet<&str>) -> CopilotLog {
    let mut log = CopilotLog::default();
    let mut variables = Variables::new();
    let mut calls: HashMap<String, Calls> = HashMap::new();
    let mut results: HashMap<String, u64> = HashMap::new();
    let mut records: Vec<Record> = Vec::new();
    for (number, line) in text.lines().enumerate().filter(|(_, line)| {
        line.contains("\"session.usage_record\"")
            || line.contains("\"assistant.message\"")
            || line.contains("\"tool.execution_complete\"")
    }) {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            log.skipped.malformed += 1;
            continue;
        };
        let body = &data["data"];
        match data["type"].as_str() {
            Some("assistant.message") => {
                let mut message = Calls::default();
                for request in body["toolRequests"].as_array().into_iter().flatten() {
                    if let Some(id) = request["toolCallId"].as_str() {
                        message.ids.push(id.to_string());
                    }
                    match (request["name"] == "bash")
                        .then(|| request["arguments"]["command"].as_str())
                        .flatten()
                    {
                        Some(command) => {
                            let command = truncate(command);
                            log.claims.extend(run_claims(command, ids, &mut variables));
                            message.commands.push(command.to_string());
                        }
                        None => message.other_tool = true,
                    }
                }
                if let Some(call) = body["apiCallId"].as_str() {
                    let entry = calls.entry(call.to_string()).or_default();
                    entry.commands.extend(message.commands);
                    entry.other_tool |= message.other_tool;
                    entry.ids.extend(message.ids);
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
                let (Some(model), Some(ts), Some(tokens)) = (
                    model_name(&usage["model"]),
                    timestamp(data.get("timestamp")),
                    copilot_tokens(usage),
                ) else {
                    log.skipped.unusable += 1;
                    continue;
                };
                let call = usage["apiCallId"].as_str().map(str::to_string);
                let key = match &call {
                    Some(call) => format!("copilot|{call}"),
                    None => {
                        log.skipped.unidentified += 1;
                        format!("copilot|{session}|line {number}")
                    }
                };
                records.push(Record {
                    key,
                    call,
                    ts,
                    model,
                    effort: effort_name(&usage["reasoningEffort"]),
                    tokens,
                    subagent: usage["initiator"] == "sub-agent",
                });
            }
            _ => {}
        }
    }
    log.responses = join_calls(records, &calls, &results, session);
    log
}

/// The run ids in `ids` that the commands of a Copilot log name.
#[cfg(test)]
pub(crate) fn copilot_claims(text: &str, ids: &HashSet<&str>) -> HashSet<String> {
    parse_copilot(text, "", ids).claims
}

/// What is kept of a session that belongs to a run, until the responses are de-duplicated.
struct Kept {
    id: String,
    claims: HashSet<String>,
    skills: Vec<(u64, String)>,
    attention: Vec<(u64, crate::attention::Signal)>,
}

/// Reads the Copilot sessions under `root`: every session that executed a Jidoka command naming a run in the
/// window, or was recorded for one.
pub(crate) fn read_copilot(
    root: &Path,
    window: &RunWindow,
    deadline: Instant,
    limits: ReadLimits,
) -> LogRead {
    let mut out = LogRead {
        root_found: root.is_dir(),
        ..LogRead::default()
    };
    if !out.root_found {
        return out;
    }
    let mut pass = ReadPass::new(window, deadline, limits);
    // The reading flags and notes of the shared helpers; moved into `out` at the end.
    let mut state = ReadResult::default();
    // A response that two logs hold (a resumed session copies its history) counts once.
    let mut seen: BTreeMap<String, Response> = BTreeMap::new();
    let mut sessions: Vec<Kept> = Vec::new();
    for dir in sub_dirs(root) {
        let session = dir.file_name().to_string_lossy().into_owned();
        let path = dir.path().join("events.jsonl");
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        if pass.over_limit(&mut state) {
            break;
        }
        if pass.stale(&path) {
            continue;
        }
        let Some(text) = pass.load(&path, &session, &mut state) else {
            continue;
        };
        let parsed = parse_copilot(&text, &session, &window.ids);
        if parsed.claims.is_empty() && !window.recorded.contains(session.as_str()) {
            continue;
        }
        let skills = copilot_skills(&text);
        let attention = crate::attention::signals(crate::attention::Agent::Copilot, &text);
        drop(text);
        if !plain_id(&session) {
            continue;
        }
        skipped_notes(
            "Copilot",
            &session,
            &parsed.skipped,
            parsed.responses.is_empty(),
            &mut state.notes,
        );
        for (key, response) in parsed.responses {
            match seen.get_mut(&key) {
                Some(existing) => merge_largest(&mut existing.tokens, &response.tokens),
                None => {
                    seen.insert(key, response);
                }
            }
        }
        sessions.push(Kept {
            id: session,
            claims: parsed.claims,
            skills,
            attention,
        });
    }
    let mut by_session: BTreeMap<String, Vec<Response>> = BTreeMap::new();
    for response in seen.into_values() {
        by_session
            .entry(response.session.clone())
            .or_default()
            .push(response);
    }
    for Kept {
        id,
        claims,
        mut skills,
        attention,
    } in sessions
    {
        let mut responses = by_session.remove(&id).unwrap_or_default();
        responses.sort_by_key(|response| response.ts);
        skills.sort();
        skills.dedup();
        out.sessions.push(LogSession {
            id,
            has_subagent_logs: responses.iter().any(|response| response.subagent),
            claims,
            responses,
            pauses: Vec::new(),
            activity: Vec::new(),
            attention,
            skills,
        });
    }
    out.notes = state.notes;
    out.timed_out = state.timed_out;
    out.truncated = state.truncated;
    out
}
