//! The Codex adapter: rollout files under `sessions/<year>/<month>/<day>/rollout-*.jsonl`, and the rollouts of
//! a session's subagents, which name their root session.
//!
//! Codex runs commands through a small JavaScript snippet (`tools.exec_command({cmd: "..."})`). The snippet is
//! scanned for string literals and never run. The command text is handed on only to be classified.

use super::*;

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
            '/' if skip_comment(&chars, i).is_some() => {
                i = skip_comment(&chars, i).unwrap_or(i + 1)
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
            '/' if skip_comment(chars, i).is_some() => {
                i = skip_comment(chars, i).unwrap_or(i + 1);
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

/// The index after the comment that starts at `chars[i]` (`// ...` to the line end, or `/* ... */`), if one does.
fn skip_comment(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i) != Some(&'/') {
        return None;
    }
    match chars.get(i + 1) {
        Some('/') => {
            let end = chars[i..].iter().position(|c| *c == '\n');
            Some(end.map_or(chars.len(), |p| i + p))
        }
        Some('*') => {
            let mut j = i + 2;
            while j < chars.len() && !(chars[j] == '*' && chars.get(j + 1) == Some(&'/')) {
                j += 1;
            }
            Some((j + 2).min(chars.len()))
        }
        _ => None,
    }
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
    loop {
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        match skip_comment(chars, i) {
            Some(next) => i = next,
            None => return i,
        }
    }
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
            '/' if skip_comment(chars, i).is_some() => {
                i = skip_comment(chars, i).unwrap_or(i + 1);
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

// --- the commands of a rollout ----------------------------------------------------------------------

/// The tool calls of the response being read, until its `token_usage_record` closes it.
#[derive(Default)]
struct Open {
    commands: Vec<String>,
    other_tool: bool,
    any_tool: bool,
    tool_output_bytes: u64,
}

/// The commands the agent ran, in order: `exec` snippets, `CommandExecution` items, and the older
/// `function_call` and `local_shell_call` shapes. Tool output, messages and file contents are not commands.
fn commands_of(data: &Value) -> Vec<String> {
    let payload = &data["payload"];
    let words = |value: &Value| match value {
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Value::String(text) => text.clone(),
        _ => String::new(),
    };
    let last_word = |value: &Value| match value {
        Value::Array(parts) => parts
            .last()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        other => words(other),
    };
    match (data["type"].as_str(), payload["type"].as_str()) {
        (Some("event_msg"), Some("item_started" | "item_completed"))
            if payload["item"]["type"] == "CommandExecution" =>
        {
            // The last element of the command array is the script; the others are the shell and its flags.
            vec![last_word(&payload["item"]["command"])]
        }
        (Some("response_item"), Some("custom_tool_call")) if payload["name"] == "exec" => {
            read_snippet(payload["input"].as_str().unwrap_or("")).commands
        }
        (Some("response_item"), Some("function_call")) => {
            let arguments: Value =
                serde_json::from_str(payload["arguments"].as_str().unwrap_or(""))
                    .unwrap_or_default();
            let command = if arguments["cmd"].is_null() {
                &arguments["command"]
            } else {
                &arguments["cmd"]
            };
            vec![words(command)]
        }
        (Some("response_item"), Some("local_shell_call")) => {
            vec![last_word(&payload["action"]["command"])]
        }
        _ => Vec::new(),
    }
}

/// The run ids in `ids` that the commands of a Codex rollout name, with shell variables carried across the
/// commands in order. Both Codex readers decide from this which runs a session belongs to.
pub(crate) fn codex_claims(text: &str, ids: &HashSet<&str>) -> HashSet<String> {
    const MARKERS: [&str; 4] = [
        "CommandExecution",
        "custom_tool_call",
        "function_call",
        "local_shell_call",
    ];
    let mut claims = HashSet::new();
    let mut variables = Variables::new();
    for line in text
        .lines()
        .filter(|line| MARKERS.iter().any(|marker| line.contains(marker)))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for command in commands_of(&data) {
            claims.extend(run_claims(truncate(&command), ids, &mut variables));
        }
    }
    claims
}

/// The ids on a rollout's `session_meta` line. A rollout of a subagent has its own id and the id of the
/// root session it belongs to.
pub(crate) fn codex_meta(text: &str) -> Option<(String, String, bool)> {
    let line = text
        .lines()
        .find(|line| line.contains("\"session_meta\""))?;
    let data: Value = serde_json::from_str(line).ok()?;
    let payload = data.get("payload")?;
    let id = payload.get("id")?.as_str()?.to_string();
    let root = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_string();
    let subagent = payload
        .pointer("/source/subagent")
        .is_some_and(|v| !v.is_null() && *v != Value::Bool(false))
        || payload["thread_source"] == "subagent"
        || root != id;
    Some((id, root, subagent))
}

/// What a Codex `turn_context` line says about the model and effort of a turn.
#[derive(Clone)]
struct TurnSettings {
    model: String,
    effort: Option<String>,
}

/// Whether a `token_usage_record` usage object reports any input or output count.
pub(crate) fn has_token_counts(usage: &Value) -> bool {
    usage.is_object()
        && (count(usage.get("input_tokens")).is_some()
            || count(usage.get("output_tokens")).is_some())
}

/// The token counts of a Codex `token_usage_record`. Codex counts cached input inside `input_tokens`; the
/// cached part is moved to cache read, and cache writes are taken out of input too.
pub(crate) fn codex_tokens(usage: &Value) -> Tokens {
    let (input, cached, written) = (
        count(usage.get("input_tokens")),
        count(usage.get("cached_input_tokens")),
        count(usage.get("cache_write_input_tokens")),
    );
    Tokens {
        input: input.map(|n| {
            n.saturating_sub(cached.unwrap_or(0))
                .saturating_sub(written.unwrap_or(0))
        }),
        cache_read: cached,
        cache_write: written,
        output: count(usage.get("output_tokens")),
        reasoning: count(usage.get("reasoning_output_tokens")),
    }
}

/// The skills a Codex rollout shows used: tool calls whose command reads a `<name>/SKILL.md`. Tool output
/// and the session's own instructions mention skill files too, so only the calls are looked at.
fn codex_skills(text: &str, parsed: &mut Markers) {
    for line in text.lines().filter(|line| line.contains("SKILL.md")) {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = &data["payload"];
        if !matches!(
            payload["type"].as_str(),
            Some("function_call" | "custom_tool_call")
        ) {
            continue;
        }
        let Some(ts) = timestamp(data.get("timestamp")) else {
            continue;
        };
        // A call's command is its `arguments` (a JSON string) or, for a custom tool, its `input`.
        let command = payload["arguments"]
            .as_str()
            .or_else(|| payload["input"].as_str())
            .unwrap_or_default();
        parsed
            .skills
            .extend(skills_read_by(command).into_iter().map(|name| (ts, name)));
    }
}

// --- the rollout files ------------------------------------------------------------------------------

/// What one rollout file gave.
#[derive(Default)]
struct CodexLog {
    /// The responses and their keys.
    responses: Vec<(String, Response)>,
    /// The run ids an executed Jidoka command in this rollout names.
    claims: HashSet<String>,
    skipped: Skipped,
    skills: Vec<(u64, String)>,
}

/// Adds one `response_item` to the open response: a call, or the result of an earlier call.
fn response_item(
    payload: &Value,
    open: &mut Open,
    open_calls: &mut HashSet<String>,
    owners: &mut HashMap<String, usize>,
    log: &mut CodexLog,
) {
    match payload["type"].as_str() {
        Some("custom_tool_call") => {
            open.any_tool = true;
            if let Some(call) = payload["call_id"].as_str() {
                open_calls.insert(call.to_string());
            }
            if payload["name"] == "exec" {
                let snippet = read_snippet(payload["input"].as_str().unwrap_or(""));
                open.commands.extend(snippet.commands);
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
                return;
            };
            let bytes = result_bytes(&payload["output"]);
            if open_calls.remove(call) {
                open.tool_output_bytes = open.tool_output_bytes.saturating_add(bytes);
            } else if let Some(index) = owners.remove(call)
                && let Some((_, response)) = log.responses.get_mut(index)
            {
                response.tool_output_bytes = Some(
                    response
                        .tool_output_bytes
                        .unwrap_or(0)
                        .saturating_add(bytes),
                );
            }
        }
        _ => {}
    }
}

/// Reads the responses of one rollout. A response ends at its `token_usage_record`; the calls since the
/// previous record or the start of the turn belong to it, and their results, which come after it, are added
/// to it by `call_id`. Its model and effort come from the `turn_context` of its turn. Reasoning tokens are
/// part of output; see `codex_tokens` for how the counts are split.
fn parse_codex(text: &str, session: &str, subagent: bool, ids: &HashSet<&str>) -> CodexLog {
    const MARKERS: [&str; 4] = [
        "\"response_item\"",
        "\"token_usage_record\"",
        "\"task_started\"",
        "\"turn_context\"",
    ];
    let mut log = CodexLog::default();
    if !subagent {
        log.claims = codex_claims(text, ids);
    }
    let mut open = Open::default();
    // The calls of the open response, and the index in `responses` of a closed response that made a call.
    let mut open_calls: HashSet<String> = HashSet::new();
    let mut owners: HashMap<String, usize> = HashMap::new();
    let mut by_turn: BTreeMap<String, TurnSettings> = BTreeMap::new();
    let mut last: Option<TurnSettings> = None;
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| MARKERS.iter().any(|marker| line.contains(marker)))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            log.skipped.malformed += 1;
            continue;
        };
        let payload = &data["payload"];
        match data["type"].as_str() {
            Some("turn_context") => {
                if let Some(model) = model_name(&payload["model"]) {
                    // The effort belongs to the same turn as the model.
                    let settings = TurnSettings {
                        model,
                        effort: effort_name(&payload["effort"]),
                    };
                    if let Some(turn) = payload["turn_id"].as_str() {
                        by_turn.insert(turn.to_string(), settings.clone());
                    }
                    last = Some(settings);
                }
            }
            // A turn starts. Calls still open belong to no response: a subagent rollout opens with a copy
            // of its parent's history, which has calls and results but no token records.
            Some("event_msg") if payload["type"] == "task_started" => {
                open = Open::default();
                open_calls.clear();
            }
            Some("response_item") => {
                response_item(payload, &mut open, &mut open_calls, &mut owners, &mut log)
            }
            Some("token_usage_record") => {
                let closed = std::mem::take(&mut open);
                let calls = std::mem::take(&mut open_calls);
                let usage = &payload["usage"];
                let ts = timestamp(data.get("timestamp")).filter(|_| has_token_counts(usage));
                // The turn's own settings, or the latest ones when the record names no known turn.
                let settings = payload["turn_id"]
                    .as_str()
                    .and_then(|turn| by_turn.get(turn))
                    .or(last.as_ref());
                let (Some(ts), Some(settings)) = (ts, settings) else {
                    log.skipped.unusable += 1;
                    continue;
                };
                let key = match payload["response_id"].as_str() {
                    Some(id) => format!("codex|{id}"),
                    None => {
                        log.skipped.unidentified += 1;
                        format!("codex|{session}|line {number}")
                    }
                };
                for call in calls {
                    owners.insert(call, log.responses.len());
                }
                log.responses.push((
                    key,
                    Response {
                        session: session.to_string(),
                        ts,
                        model: settings.model.clone(),
                        effort: settings.effort.clone(),
                        tokens: codex_tokens(usage),
                        subagent,
                        commands: closed.commands,
                        other_tool: closed.other_tool,
                        any_tool: closed.any_tool,
                        calls_known: true,
                        tool_output_bytes: Some(closed.tool_output_bytes),
                    },
                ));
            }
            _ => {}
        }
    }
    log.skills = {
        let mut markers = Markers::default();
        codex_skills(text, &mut markers);
        markers.skills
    };
    log
}

/// A rollout seen on the first pass: its ids and where it is, not its text, so unclaimed rollouts cost no
/// memory.
struct Found {
    path: PathBuf,
    id: String,
    root: String,
    subagent: bool,
    claims: HashSet<String>,
}

/// The recorded session a rollout's file name ends with (`rollout-<time>-<session id>.jsonl`). A rollout
/// that cannot be read has no `session_meta` to say whose it is, so its name does.
fn recorded_in_name<'a>(name: &str, recorded: &HashSet<&'a str>) -> Option<&'a str> {
    let stem = name.strip_suffix(".jsonl")?;
    recorded
        .iter()
        .find(|id| stem == **id || stem.ends_with(&format!("-{id}")))
        .copied()
}

/// Reads the Codex rollouts under `root`: every session that executed a Jidoka command naming a run in the
/// window, or was recorded for one, together with the rollouts of its subagents.
#[expect(
    clippy::too_many_lines,
    reason = "walks the day folders, picks the claimed roots, parses their rollouts and merges the responses in one pass"
)]
pub(crate) fn read_codex(
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
    let earliest_day = civil_day(window.from.saturating_sub(DAY_MS));
    let mut found: Vec<Found> = Vec::new();
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
                    if pass.over_limit(&mut state) {
                        break 'days;
                    }
                    if pass.stale(&path) {
                        continue;
                    }
                    let session =
                        recorded_in_name(&file_name, &window.recorded).unwrap_or(&file_name);
                    let Some(text) = pass.load(&path, session, &mut state) else {
                        continue;
                    };
                    let Some((id, session_root, subagent)) = codex_meta(&text) else {
                        continue;
                    };
                    let claims = if subagent {
                        HashSet::new()
                    } else {
                        codex_claims(&text, &window.ids)
                    };
                    found.push(Found {
                        path,
                        id,
                        root: session_root,
                        subagent,
                        claims,
                    });
                }
            }
        }
    }
    // A subagent's rollout names the run only through its root session, so the roots decide.
    let claimed_roots: HashSet<String> = found
        .iter()
        .filter(|rollout| {
            !rollout.claims.is_empty()
                || window.recorded.contains(rollout.id.as_str())
                || window.recorded.contains(rollout.root.as_str())
        })
        .map(|rollout| rollout.root.clone())
        .collect();
    struct Root {
        claims: HashSet<String>,
        has_subagent_logs: bool,
        skipped: Skipped,
        any_response: bool,
        markers: Markers,
    }
    let mut by_root: BTreeMap<String, Root> = BTreeMap::new();
    // A response that two rollouts hold counts once, with the largest counts.
    let mut seen: BTreeMap<String, Response> = BTreeMap::new();
    for rollout in found.iter().filter(|r| claimed_roots.contains(&r.root)) {
        if pass.over_limit(&mut state) {
            break;
        }
        // Only a claimed rollout is read again, to parse it.
        let Some(text) = pass.load(&rollout.path, &rollout.root, &mut state) else {
            continue;
        };
        let parsed = parse_codex(&text, &rollout.root, rollout.subagent, &window.ids);
        let attention = (!rollout.subagent)
            .then(|| crate::attention::signals(crate::agent::Agent::Codex, &text));
        drop(text);
        let entry = by_root.entry(rollout.root.clone()).or_insert_with(|| Root {
            claims: HashSet::new(),
            has_subagent_logs: false,
            skipped: Skipped::default(),
            any_response: false,
            markers: Markers::default(),
        });
        entry.claims.extend(rollout.claims.iter().cloned());
        entry.has_subagent_logs |= rollout.subagent;
        entry.skipped.add(&parsed.skipped);
        entry.any_response |= !parsed.responses.is_empty();
        entry.markers.skills.extend(parsed.skills);
        entry
            .markers
            .attention
            .extend(attention.into_iter().flatten());
        for (key, response) in parsed.responses {
            match seen.get_mut(&key) {
                Some(existing) => {
                    merge_largest(&mut existing.tokens, &response.tokens);
                    existing.tool_output_bytes =
                        existing.tool_output_bytes.max(response.tool_output_bytes);
                }
                None => {
                    seen.insert(key, response);
                }
            }
        }
    }
    let mut by_session: BTreeMap<String, Vec<Response>> = BTreeMap::new();
    for response in seen.into_values() {
        by_session
            .entry(response.session.clone())
            .or_default()
            .push(response);
    }
    for (id, root) in by_root {
        if !plain_id(&id) {
            continue;
        }
        skipped_notes(
            "Codex",
            &id,
            &root.skipped,
            !root.any_response,
            &mut state.notes,
        );
        let mut responses = by_session.remove(&id).unwrap_or_default();
        responses.sort_by_key(|response| response.ts);
        let mut skills = root.markers.skills;
        skills.sort();
        skills.dedup();
        out.sessions.push(LogSession {
            id,
            has_subagent_logs: root.has_subagent_logs,
            claims: root.claims,
            responses,
            pauses: Vec::new(),
            activity: Vec::new(),
            attention: root.markers.attention,
            skills,
        });
    }
    out.notes = state.notes;
    out.timed_out = state.timed_out;
    out.truncated = state.truncated;
    out
}
