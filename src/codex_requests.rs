//! Reads the model requests of Codex rollout files for `factory run requests`.
//!
//! Like the Claude Code reader, it keeps of each request only the time, the token counts and a category
//! taken from its commands; of each session, the runs that an executed `factory run update <id>` names.
//! Codex runs commands through a small JavaScript snippet (`tools.exec_command({cmd: "..."})`). The snippet
//! is scanned for string literals and never run. The command text, tool output and paths are dropped as
//! soon as they are classified: nothing but categories, flags and counts leaves this module.

use crate::{
    request_logs::{
        classify, result_bytes, updated_runs, Request, RequestRead, SessionRequests,
        MAX_COMMAND_BYTES,
    },
    usage_logs::{
        civil_day, codex_meta, codex_tokens, files_in, has_token_counts, merge_largest,
        modified_ms, read_text, sub_dirs, timestamp, LogFile, ReadLimits, DAY_MS, MTIME_SLACK_MS,
    },
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::Instant,
};

/// The most of one JavaScript snippet that is read. A patch can carry whole files.
const MAX_SNIPPET_CHARS: usize = 256 * 1024;

// --- the JavaScript snippet -------------------------------------------------------------------------

/// The commands and tool calls in one `exec` snippet.
#[derive(Debug, Default, PartialEq, Eq)]
struct Snippet {
    /// The `cmd` of each `tools.exec_command` call that is a plain string literal.
    commands: Vec<String>,
    /// A call that is not a readable command: another tool, a `cmd` that is not a plain literal, or a
    /// snippet that runs no tool at all.
    other_tool: bool,
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Scans a snippet for `tools.<name>(` calls outside strings and comments. `tools.exec_command` calls whose
/// `cmd` is a plain string literal give a command; every other call is a tool call that is not a command.
fn read_snippet(input: &str) -> Snippet {
    let chars: Vec<char> = input.chars().take(MAX_SNIPPET_CHARS).collect();
    let mut snippet = Snippet::default();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' | '\'' | '`' => i = skip_string(&chars, i),
            '/' if chars.get(i + 1) == Some(&'/') => {
                while chars.get(i).is_some_and(|c| *c != '\n') {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            }
            c if is_identifier_char(c) => {
                let start = i;
                while chars.get(i).is_some_and(|c| is_identifier_char(*c)) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let after_dot = chars.get(i) == Some(&'.');
                if word != "tools" || !after_dot || (start > 0 && chars[start - 1] == '.') {
                    continue;
                }
                let name_start = i + 1;
                let mut j = name_start;
                while chars.get(j).is_some_and(|c| is_identifier_char(*c)) {
                    j += 1;
                }
                let name: String = chars[name_start..j].iter().collect();
                let mut k = j;
                while chars.get(k).is_some_and(|c| c.is_whitespace()) {
                    k += 1;
                }
                if name.is_empty() || chars.get(k) != Some(&'(') {
                    continue;
                }
                if name == "exec_command" {
                    // The arguments are read within the call, and scanning goes on after it, so that
                    // every character is looked at a bounded number of times, however broken the call is.
                    let end = call_end(&chars, k + 1);
                    match exec_command_literal(&chars[..end], k + 1) {
                        Some(command) => snippet.commands.push(command),
                        None => snippet.other_tool = true,
                    }
                    i = end;
                    continue;
                } else {
                    snippet.other_tool = true;
                }
                i = k + 1;
            }
            _ => i += 1,
        }
    }
    // A snippet without a readable command still called the tool, so it is a tool call.
    if snippet.commands.is_empty() {
        snippet.other_tool = true;
    }
    snippet
}

/// The index after the `)` that closes the call whose arguments start at `chars[start]`, or the end.
fn call_end(chars: &[char], start: usize) -> usize {
    let (mut depth, mut i) = (1usize, start);
    while i < chars.len() {
        match chars[i] {
            '"' | '\'' | '`' => {
                i = skip_string(chars, i);
                continue;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

/// The index after the string literal that starts at `chars[start]` (an unterminated one runs to the end).
fn skip_string(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

fn skip_whitespace(chars: &[char], mut i: usize) -> usize {
    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    i
}

/// The `cmd` of a `tools.exec_command({...})` call whose argument list starts at `chars[start]`, when it is
/// a single plain string literal; `None` for anything else, such as a variable or a concatenation.
fn exec_command_literal(chars: &[char], start: usize) -> Option<String> {
    let mut i = skip_whitespace(chars, start);
    if chars.get(i) != Some(&'{') {
        return None;
    }
    i += 1;
    loop {
        i = skip_whitespace(chars, i);
        // A property key: an identifier or a quoted string.
        let key = match chars.get(i)? {
            '"' | '\'' => {
                let (key, next) = decode_string(chars, i)?;
                i = next;
                key
            }
            '}' => return None,
            c if is_identifier_char(*c) => {
                let from = i;
                while chars.get(i).is_some_and(|c| is_identifier_char(*c)) {
                    i += 1;
                }
                chars[from..i].iter().collect()
            }
            _ => return None,
        };
        i = skip_whitespace(chars, i);
        match chars.get(i)? {
            // `{cmd}` or `{cmd, ...}`: the value is a variable.
            ',' | '}' => {
                if key == "cmd" || chars[i] == '}' {
                    return None;
                }
                i += 1;
                continue;
            }
            ':' => i += 1,
            _ => return None,
        }
        i = skip_whitespace(chars, i);
        if key == "cmd" {
            if !matches!(chars.get(i)?, '"' | '\'' | '`') {
                return None;
            }
            let (command, next) = decode_string(chars, i)?;
            let next = skip_whitespace(chars, next);
            // Only a literal on its own: `"a" + b` is not one.
            return matches!(chars.get(next)?, ',' | '}').then_some(command);
        }
        i = skip_value(chars, i)?;
        match chars.get(i)? {
            ',' => i += 1,
            _ => return None,
        }
    }
}

/// The index of the `,` or `}` that ends the property value starting at `chars[start]`.
fn skip_value(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '"' | '\'' | '`' => {
                i = skip_string(chars, i);
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' => depth = depth.checked_sub(1)?,
            '}' if depth == 0 => return Some(i),
            '}' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Decodes the JavaScript string literal that starts at `chars[start]`, with its JavaScript escapes, and the
/// index after it. `None` when it is not closed, or a template literal holds `${...}`.
fn decode_string(chars: &[char], start: usize) -> Option<(String, usize)> {
    let quote = chars[start];
    let mut out = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        let c = chars[i];
        if c == quote {
            return Some((out, i + 1));
        }
        if quote == '`' && c == '$' && chars.get(i + 1) == Some(&'{') {
            return None;
        }
        if c != '\\' {
            if out.len() < MAX_COMMAND_BYTES {
                out.push(c);
            }
            i += 1;
            continue;
        }
        let escaped = *chars.get(i + 1)?;
        i += 2;
        let decoded = match escaped {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'v' => '\u{b}',
            '0' => '\0',
            '\n' => continue,
            '\r' => {
                if chars.get(i) == Some(&'\n') {
                    i += 1;
                }
                continue;
            }
            'x' => {
                let hex: String = chars.get(i..i + 2)?.iter().collect();
                i += 2;
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
            }
            'u' if chars.get(i) == Some(&'{') => {
                let close = chars[i..].iter().take(10).position(|c| *c == '}')? + i;
                let hex: String = chars[i + 1..close].iter().collect();
                i = close + 1;
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
            }
            'u' => {
                let hex: String = chars.get(i..i + 4)?.iter().collect();
                i += 4;
                // A surrogate pair would need its second half; a lone one cannot be a command word.
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
            }
            other => other,
        };
        if out.len() < MAX_COMMAND_BYTES {
            out.push(decoded);
        }
    }
    None
}

// --- the rollout files ------------------------------------------------------------------------------

/// The tool calls of the response being read, until its `token_usage_record` closes it.
#[derive(Default)]
struct Open {
    commands: Vec<String>,
    other_tool: bool,
    any_tool: bool,
    tool_output_bytes: u64,
}

/// What one rollout file gave.
struct Rollout {
    /// The responses, by key.
    requests: Vec<(String, Request)>,
    /// The run ids an executed `factory run update` in this rollout names.
    claims: HashSet<String>,
    malformed: usize,
}

/// Reads the responses of one rollout. A response ends at its `token_usage_record`; the calls since the
/// previous record or the start of the turn belong to it, and their results, which come after it, are added
/// to it by `call_id`.
fn parse_rollout(text: &str, session: &str, subagent: bool, ids: &HashSet<&str>) -> Rollout {
    let mut rollout = Rollout {
        requests: Vec::new(),
        claims: HashSet::new(),
        malformed: 0,
    };
    let mut open = Open::default();
    // The calls of the open response, and the index in `requests` of a closed response that made a call.
    let mut open_calls: HashSet<String> = HashSet::new();
    let mut owners: HashMap<String, usize> = HashMap::new();
    for (number, line) in text.lines().enumerate().filter(|(_, line)| {
        line.contains("\"response_item\"")
            || line.contains("\"token_usage_record\"")
            || line.contains("\"task_started\"")
    }) {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            rollout.malformed += 1;
            continue;
        };
        let payload = &data["payload"];
        match data["type"].as_str() {
            // A turn starts. Calls still open belong to no response: a subagent rollout opens with a copy
            // of its parent's history, which has calls and results but no token records.
            Some("event_msg") if payload["type"] == "task_started" => {
                open = Open::default();
                open_calls.clear();
            }
            Some("response_item") => match payload["type"].as_str() {
                Some("custom_tool_call") => {
                    open.any_tool = true;
                    if let Some(call) = payload["call_id"].as_str() {
                        open_calls.insert(call.to_string());
                    }
                    if payload["name"] == "exec" {
                        let snippet = read_snippet(payload["input"].as_str().unwrap_or(""));
                        for command in snippet.commands {
                            if !subagent {
                                rollout.claims.extend(updated_runs(&command, ids));
                            }
                            open.commands.push(command);
                        }
                        open.other_tool |= snippet.other_tool;
                    } else {
                        open.other_tool = true;
                    }
                }
                Some("function_call") => {
                    open.any_tool = true;
                    open.other_tool = true;
                    if let Some(call) = payload["call_id"].as_str() {
                        open_calls.insert(call.to_string());
                    }
                }
                Some("custom_tool_call_output" | "function_call_output") => {
                    let Some(call) = payload["call_id"].as_str() else {
                        continue;
                    };
                    let bytes = result_bytes(&payload["output"]);
                    if open_calls.remove(call) {
                        open.tool_output_bytes = open.tool_output_bytes.saturating_add(bytes);
                    } else if let Some(index) = owners.remove(call) {
                        if let Some((_, request)) = rollout.requests.get_mut(index) {
                            request.tool_output_bytes =
                                Some(request.tool_output_bytes.unwrap_or(0).saturating_add(bytes));
                        }
                    }
                }
                _ => {}
            },
            Some("token_usage_record") => {
                let closed = std::mem::take(&mut open);
                let calls = std::mem::take(&mut open_calls);
                let usage = &payload["usage"];
                let tokens = codex_tokens(usage);
                let ts = timestamp(data.get("timestamp"));
                let Some(ts) = ts.filter(|_| has_token_counts(usage)) else {
                    continue;
                };
                let class = classify(&closed.commands, closed.other_tool, closed.any_tool);
                let key = match payload["response_id"].as_str() {
                    Some(id) => format!("codex|{id}"),
                    None => format!("codex|{session}|line {number}"),
                };
                for call in calls {
                    owners.insert(call, rollout.requests.len());
                }
                rollout.requests.push((
                    key,
                    Request {
                        ts,
                        category: class.category,
                        tokens,
                        chained_reporting: class.chained_reporting,
                        several_categories: class.several_categories,
                        tool_output_bytes: Some(closed.tool_output_bytes),
                        subagent,
                    },
                ));
            }
            _ => {}
        }
    }
    rollout
}

/// A rollout file seen on the first pass.
struct Seen {
    path: PathBuf,
    id: String,
    root: String,
    subagent: bool,
    /// Parsed already, because it names a tracked run or was recorded.
    parsed: Option<Rollout>,
}

/// Reads the Codex rollouts under `root`: every session that executed `factory run update` for a run in
/// `ids`, or was recorded for one in `recorded`, together with the rollouts of its subagents.
pub(crate) fn read_codex_requests(
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
    let earliest_day = civil_day(from.saturating_sub(DAY_MS));
    let mut seen: Vec<Seen> = Vec::new();
    'days: for year in sub_dirs(root) {
        for month in sub_dirs(&year.path()) {
            for day in sub_dirs(&month.path()) {
                let name = format!(
                    "{}-{}-{}",
                    year.file_name().to_string_lossy(),
                    month.file_name().to_string_lossy(),
                    day.file_name().to_string_lossy()
                );
                if name < earliest_day {
                    continue;
                }
                for file in files_in(&day.path()) {
                    let path = file.path();
                    let file_name = file.file_name().to_string_lossy().into_owned();
                    if !(file_name.starts_with("rollout-") && file_name.ends_with(".jsonl")) {
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
                        break 'days;
                    }
                    if modified_ms(&path).is_some_and(|m| m.saturating_add(MTIME_SLACK_MS) < from) {
                        continue;
                    }
                    let text = match read_text(&path) {
                        Ok(LogFile::Content(text)) => text,
                        Ok(LogFile::Skipped | LogFile::TooBig) | Err(_) => continue,
                    };
                    bytes += text.len() as u64;
                    let Some((id, session_root, subagent)) = codex_meta(&text) else {
                        continue;
                    };
                    let is_recorded =
                        recorded.contains(id.as_str()) || recorded.contains(session_root.as_str());
                    // Only a rollout that names a tracked run, or was recorded, is read now.
                    let parsed = (is_recorded || ids.iter().any(|run| text.contains(run)))
                        .then(|| parse_rollout(&text, &session_root, subagent, ids));
                    seen.push(Seen {
                        path,
                        id,
                        root: session_root,
                        subagent,
                        parsed,
                    });
                }
            }
        }
    }
    // A subagent's rollout names the run only through its root session, so the roots decide.
    let claimed_roots: HashSet<String> = seen
        .iter()
        .filter(|rollout| {
            recorded.contains(rollout.id.as_str())
                || recorded.contains(rollout.root.as_str())
                || (!rollout.subagent
                    && rollout
                        .parsed
                        .as_ref()
                        .is_some_and(|parsed| !parsed.claims.is_empty()))
        })
        .map(|rollout| rollout.root.clone())
        .collect();
    let mut malformed = 0;
    let mut by_root: BTreeMap<String, (HashSet<String>, bool)> = BTreeMap::new();
    // A response that two rollouts hold counts once, with the largest counts.
    let mut requests: BTreeMap<String, (String, Request)> = BTreeMap::new();
    for rollout in seen.into_iter().filter(|r| claimed_roots.contains(&r.root)) {
        if Instant::now() > deadline {
            out.timed_out = true;
            break;
        }
        let parsed = match rollout.parsed {
            Some(parsed) => parsed,
            None => match read_text(&rollout.path) {
                Ok(LogFile::Content(text)) => {
                    parse_rollout(&text, &rollout.root, rollout.subagent, ids)
                }
                _ => continue,
            },
        };
        malformed += parsed.malformed;
        let entry = by_root.entry(rollout.root.clone()).or_default();
        entry.0.extend(parsed.claims);
        entry.1 |= rollout.subagent;
        for (key, request) in parsed.requests {
            match requests.get_mut(&key) {
                Some((_, existing)) => {
                    merge_largest(&mut existing.tokens, &request.tokens);
                    existing.tool_output_bytes =
                        existing.tool_output_bytes.max(request.tool_output_bytes);
                }
                None => {
                    requests.insert(key, (rollout.root.clone(), request));
                }
            }
        }
    }
    if malformed > 0 {
        out.notes.push(format!(
            "{malformed} malformed Codex log line(s) were skipped"
        ));
    }
    let mut by_session: BTreeMap<String, Vec<Request>> = BTreeMap::new();
    for (session, request) in requests.into_values() {
        by_session.entry(session).or_default().push(request);
    }
    for (id, (claims, has_subagent_logs)) in by_root {
        let mut requests = by_session.remove(&id).unwrap_or_default();
        requests.sort_by_key(|request| request.ts);
        out.sessions.push(SessionRequests {
            id,
            has_subagent_logs,
            claims,
            requests,
        });
    }
    out
}
