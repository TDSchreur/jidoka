//! The Claude Code adapter: transcripts under `projects/<slug>/<session>.jsonl` and the subagent transcripts in
//! `<session>/subagents/`.

use super::*;

/// What one Claude Code log file gave. Lines that looked like a response but could not be used are
/// counted, so the notes can say so instead of silently looking complete.
#[derive(Default)]
struct ClaudeLog {
    /// Responses by key; Claude Code writes one per content block, so a response counts once.
    responses: BTreeMap<String, Response>,
    claims: HashSet<String>,
    skipped: Skipped,
}

/// The tool calls of one Claude Code line.
struct ToolCalls {
    /// The Bash commands, truncated.
    commands: Vec<String>,
    /// The ids of the calls, to match their results.
    ids: Vec<String>,
    any_tool: bool,
    other_tool: bool,
}

/// The tool calls in a line's content blocks. When `claiming`, the run ids their Bash commands name are
/// added to `claims`: a coordinator's executed command claims, a subagent's does not.
fn tool_calls(
    blocks: &[Value],
    claiming: bool,
    claims: &mut HashSet<String>,
    ids: &HashSet<&str>,
    variables: &mut Variables,
) -> ToolCalls {
    let mut calls = ToolCalls {
        commands: Vec::new(),
        ids: Vec::new(),
        any_tool: false,
        other_tool: false,
    };
    for block in blocks.iter().filter(|b| b["type"] == "tool_use") {
        calls.any_tool = true;
        if let Some(id) = block["id"].as_str() {
            calls.ids.push(id.to_string());
        }
        let command = (block["name"] == "Bash")
            .then(|| block["input"]["command"].as_str())
            .flatten();
        match command {
            Some(command) => {
                let command = truncate(command);
                if claiming {
                    claims.extend(run_claims(command, ids, variables));
                }
                calls.commands.push(command.to_string());
            }
            None => calls.other_tool = true,
        }
    }
    calls
}

/// The responses and claims of one Claude Code log file. `input_tokens` excludes the cache; thinking tokens
/// are inside `output_tokens`. A usage-limit message and a virtual line (a tool called from inside a script
/// tool) carry zero usage under a made-up model: they are not responses. A virtual line's Bash command was
/// still executed, so it can claim a run.
fn parse_claude(text: &str, session: &str, subagent_file: bool, ids: &HashSet<&str>) -> ClaudeLog {
    let mut parsed = ClaudeLog::default();
    let mut variables = Variables::new();
    // Which response made the call a result answers; each call's result counts once.
    let mut owners: HashMap<String, String> = HashMap::new();
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("\"usage\"") || line.contains("\"tool_result\""))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            parsed.skipped.malformed += 1;
            continue;
        };
        let subagent = subagent_file || data["isSidechain"].as_bool() == Some(true);
        let message = &data["message"];
        let blocks = message["content"].as_array().map_or(&[][..], Vec::as_slice);
        for block in blocks.iter().filter(|b| b["type"] == "tool_result") {
            if let Some(key) = block["tool_use_id"]
                .as_str()
                .and_then(|id| owners.remove(id))
                && let Some(response) = parsed.responses.get_mut(&key)
            {
                response.tool_output_bytes = Some(
                    response
                        .tool_output_bytes
                        .unwrap_or(0)
                        .saturating_add(result_bytes(&block["content"])),
                );
            }
        }
        let not_a_response = data["isApiErrorMessage"].as_bool() == Some(true)
            || data["isVirtual"].as_bool() == Some(true);
        let calls = tool_calls(blocks, !subagent, &mut parsed.claims, ids, &mut variables);
        let usage = &message["usage"];
        // Another kind of line that mentions "usage", such as a task's totals, is not a response either.
        if not_a_response || !usage.is_object() {
            continue;
        }
        let (Some(model), Some(ts)) = (
            model_name(&message["model"]),
            timestamp(data.get("timestamp")),
        ) else {
            parsed.skipped.unusable += 1;
            continue;
        };
        let tokens = Tokens {
            input: count(usage.get("input_tokens")),
            cache_read: count(usage.get("cache_read_input_tokens")),
            cache_write: count(usage.get("cache_creation_input_tokens")),
            output: count(usage.get("output_tokens")),
            reasoning: count(usage.pointer("/output_tokens_details/thinking_tokens")),
        };
        if tokens.input.is_none() && tokens.output.is_none() {
            parsed.skipped.unusable += 1;
            continue;
        }
        let key = match message["id"].as_str().or(data["uuid"].as_str()) {
            Some(id) => format!("claude|{id}|{}", data["requestId"].as_str().unwrap_or("")),
            None => {
                parsed.skipped.unidentified += 1;
                format!("claude|{session}|line {number}")
            }
        };
        let response = parsed
            .responses
            .entry(key.clone())
            .or_insert_with(|| Response {
                session: session.to_string(),
                ts,
                model,
                effort: effort_name(&data["effort"]),
                tokens: Tokens::default(),
                subagent,
                commands: Vec::new(),
                other_tool: false,
                any_tool: false,
                tool_output_bytes: Some(0),
            });
        merge_largest(&mut response.tokens, &tokens);
        response.commands.extend(calls.commands);
        response.other_tool |= calls.other_tool;
        response.any_tool |= calls.any_tool;
        for id in calls.ids {
            owners.insert(id, key.clone());
        }
    }
    parsed
}

/// The pause markers and the moments of activity in a Claude Code transcript; only timestamps are kept.
/// A usage limit is an assistant line flagged `isApiErrorMessage` with error `rate_limit`; a compaction is
/// a `compact_boundary` system line. Other lines of the user or the assistant are activity, except an
/// error message and the summary a compaction writes.
fn claude_pauses(text: &str, parsed: &mut LogUsage) {
    for line in text.lines().filter(|line| {
        line.contains("\"type\":\"user\"")
            || line.contains("\"type\":\"assistant\"")
            || line.contains("compact_boundary")
    }) {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(ts) = timestamp(data.get("timestamp")) else {
            continue;
        };
        match data["type"].as_str() {
            Some("system") if data["subtype"].as_str() == Some("compact_boundary") => {
                parsed.pauses.push((PauseKind::Compaction, ts));
            }
            Some("assistant") if data["isApiErrorMessage"].as_bool() == Some(true) => {
                if data["error"].as_str() == Some("rate_limit") {
                    parsed.pauses.push((PauseKind::UsageLimit, ts));
                }
            }
            Some("user" | "assistant") if data["isCompactSummary"].as_bool() != Some(true) => {
                parsed.activity.push(ts);
            }
            _ => {}
        }
    }
}

/// The skills a Claude Code transcript shows used: assistant `Skill` tool calls. Only names and times are kept.
fn claude_skills(text: &str, parsed: &mut LogUsage) {
    for line in text.lines().filter(|line| line.contains("\"Skill\"")) {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (Some(ts), Some(content)) = (
            timestamp(data.get("timestamp")),
            data["message"]["content"].as_array(),
        ) else {
            continue;
        };
        for call in content
            .iter()
            .filter(|c| c["type"] == "tool_use" && c["name"] == "Skill")
        {
            if let Some(name) = call["input"]["skill"].as_str().and_then(skill_name) {
                parsed.skills.push((ts, name));
            }
        }
    }
}

/// Reads the Claude Code logs under `root`: every session that executed a Jidoka command naming a run in
/// the window, or was recorded for one, together with the logs of its subagents.
#[expect(
    clippy::too_many_lines,
    reason = "walks the session folders, parses each log with its subagents and merges the responses in one pass"
)]
pub(crate) fn read_claude(
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
    // A response that two session files hold (a resumed session copies its history) counts once.
    let mut seen: BTreeMap<String, Response> = BTreeMap::new();
    let mut kept: BTreeMap<String, LogUsage> = BTreeMap::new();
    let mut claims: BTreeMap<String, (HashSet<String>, bool)> = BTreeMap::new();
    'logs: for slug in sub_dirs(root) {
        for file in files_in(&slug.path()) {
            let path = file.path();
            let Some(stem) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".jsonl"))
                .map(str::to_string)
            else {
                continue;
            };
            if pass.over_limit(&mut state) {
                break 'logs;
            }
            if pass.stale(&path) {
                continue;
            }
            let Some(text) = pass.load(&path, &stem, &mut state) else {
                continue;
            };
            let mut parsed = parse_claude(&text, &stem, false, &window.ids);
            if parsed.claims.is_empty() && !window.recorded.contains(stem.as_str()) {
                continue;
            }
            let mut markers = LogUsage::default();
            claude_pauses(&text, &mut markers);
            claude_skills(&text, &mut markers);
            markers.attention = crate::attention::signals(crate::attention::Agent::Claude, &text);
            drop(text);
            // The subagents of a session are in its own folder, which is not followed if it is a link.
            let session_dir = slug.path().join(&stem);
            let subagent_dir = session_dir.join("subagents");
            let mut has_subagent_logs = false;
            if is_real_dir(&session_dir) && is_real_dir(&subagent_dir) {
                for sub in files_in(&subagent_dir) {
                    let sub_path = sub.path();
                    if sub_path.extension().is_none_or(|e| e != "jsonl") {
                        continue;
                    }
                    has_subagent_logs = true;
                    if pass.over_limit(&mut state) {
                        break;
                    }
                    if let Some(sub_text) = pass.load(&sub_path, &stem, &mut state) {
                        let more = parse_claude(&sub_text, &stem, true, &window.ids);
                        parsed.skipped.add(&more.skipped);
                        for (key, response) in more.responses {
                            parsed.responses.entry(key).or_insert(response);
                        }
                        let mut sub_markers = LogUsage::default();
                        claude_pauses(&sub_text, &mut sub_markers);
                        claude_skills(&sub_text, &mut sub_markers);
                        markers.absorb(sub_markers);
                    }
                }
            }
            if !plain_id(&stem) {
                continue;
            }
            skipped_notes(
                "Claude Code",
                &stem,
                &parsed.skipped,
                parsed.responses.is_empty(),
                &mut state.notes,
            );
            for (key, response) in parsed.responses {
                match seen.get_mut(&key) {
                    Some(existing) => {
                        merge_largest(&mut existing.tokens, &response.tokens);
                        existing.commands.extend(response.commands);
                        existing.other_tool |= response.other_tool;
                        existing.any_tool |= response.any_tool;
                        existing.tool_output_bytes =
                            existing.tool_output_bytes.max(response.tool_output_bytes);
                    }
                    None => {
                        seen.insert(key, response);
                    }
                }
            }
            let entry = claims.entry(stem.clone()).or_default();
            entry.0.extend(parsed.claims);
            entry.1 |= has_subagent_logs;
            kept.entry(stem).or_default().absorb(markers);
        }
    }
    let mut by_session: BTreeMap<String, Vec<Response>> = BTreeMap::new();
    for response in seen.into_values() {
        by_session
            .entry(response.session.clone())
            .or_default()
            .push(response);
    }
    for (id, (claims, has_subagent_logs)) in claims {
        let mut responses = by_session.remove(&id).unwrap_or_default();
        responses.sort_by_key(|response| response.ts);
        let mut markers = kept.remove(&id).unwrap_or_default();
        markers.pauses.sort_by_key(|(_, at)| *at);
        markers.activity.sort_unstable();
        markers.activity.dedup();
        markers.skills.sort();
        markers.skills.dedup();
        out.sessions.push(LogSession {
            id,
            has_subagent_logs,
            claims,
            responses,
            pauses: markers.pauses,
            activity: markers.activity,
            attention: markers.attention,
            skills: markers.skills,
        });
    }
    out.notes = state.notes;
    out.timed_out = state.timed_out;
    out.truncated = state.truncated;
    out
}
