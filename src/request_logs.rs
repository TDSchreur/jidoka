//! Reads the model requests of Claude Code session logs for `jidoka run requests`.
//!
//! Of each request it keeps the time, the token counts and a category taken from the Bash commands of its
//! tool calls; of each session, the runs that an executed `jidoka run update <id>` command names. The
//! command text, tool output, prompts and paths are dropped as soon as they are classified: nothing but
//! categories, flags and counts leaves this module. The usage reader keeps its own promise that no tool
//! text leaves it.

use crate::{
    usage::Tokens,
    usage_logs::{
        LogFile, MTIME_SLACK_MS, ReadLimits, count, files_in, merge_largest, model_name,
        modified_ms, read_text, sub_dirs, timestamp,
    },
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::Path,
    time::Instant,
};

pub(crate) const MAX_COMMAND_BYTES: usize = 64 * 1024;

/// What a model request did. The order is the priority: a request that fits several categories takes the
/// first one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Category {
    Workspace,
    Commit,
    PrCreationAndPush,
    PrSynchronization,
    CiWatch,
    Verification,
    ContinuationCheck,
    /// Its only command updates the run.
    StandaloneReporting,
    /// Any other tool call.
    ReadingEditingAnalysis,
    /// No tool call.
    TextOnly,
    /// A Copilot request whose log has no assistant message to read its tool calls from.
    NotCategorized,
}

impl Category {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Category::Workspace => "workspace",
            Category::Commit => "commit",
            Category::PrCreationAndPush => "PR creation and push",
            Category::PrSynchronization => "PR synchronization",
            Category::CiWatch => "CI watch",
            Category::Verification => "verification",
            Category::ContinuationCheck => "continuation check",
            Category::StandaloneReporting => "standalone reporting",
            Category::ReadingEditingAnalysis => "reading, editing, analysis",
            Category::TextOnly => "text only",
            Category::NotCategorized => "not categorized",
        }
    }
}

/// One model request.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) ts: u64,
    pub(crate) category: Category,
    pub(crate) tokens: Tokens,
    /// Its commands also updated the run, next to other work.
    pub(crate) chained_reporting: bool,
    /// Its commands fit more than one category.
    pub(crate) several_categories: bool,
    /// The bytes of text its tool calls returned; unknown when the log does not say.
    pub(crate) tool_output_bytes: Option<u64>,
    /// Whether a subagent made it.
    pub(crate) subagent: bool,
}

/// The requests of one session log, and the runs it claims.
#[derive(Debug)]
pub(crate) struct SessionRequests {
    pub(crate) id: String,
    /// Whether the session has subagent logs. Without them its figures cover the coordinator only.
    pub(crate) has_subagent_logs: bool,
    /// The run ids an executed `jidoka run update <id>` command in this session names.
    pub(crate) claims: HashSet<String>,
    pub(crate) requests: Vec<Request>,
}

#[derive(Debug, Default)]
pub(crate) struct RequestRead {
    pub(crate) sessions: Vec<SessionRequests>,
    pub(crate) notes: Vec<String>,
    pub(crate) root_found: bool,
    pub(crate) timed_out: bool,
    pub(crate) truncated: bool,
}

impl RequestRead {
    /// Adds what another root held. Notes already present are not repeated. Returns whether reading
    /// should stop, because a time or size limit was hit.
    pub(crate) fn absorb(&mut self, other: RequestRead) -> bool {
        self.sessions.extend(other.sessions);
        for note in other.notes {
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
        }
        self.root_found |= other.root_found;
        self.timed_out |= other.timed_out;
        self.truncated |= other.truncated;
        other.timed_out || other.truncated
    }
}

/// What one response said so far. Claude Code writes a response once per content block, so the commands
/// of a request are spread over several lines with the same message id.
#[derive(Default)]
struct Pending {
    ts: u64,
    tokens: Tokens,
    commands: Vec<String>,
    other_tool: bool,
    any_tool: bool,
    tool_output_bytes: u64,
    subagent: bool,
}

/// Reads the Claude Code logs under `root`: every session that executed `jidoka run update` for a run in
/// `ids`, or was recorded for one in `recorded`, together with the logs of its subagents.
#[expect(
    clippy::too_many_lines,
    reason = "scans Claude logs, filters by run, and tallies usage in one pass; six inputs"
)]
pub(crate) fn read_requests(
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
    // A response that two session files hold (a resumed session copies its history) counts once.
    let mut seen: BTreeMap<String, (String, Pending)> = BTreeMap::new();
    let mut kept: BTreeMap<String, (HashSet<String>, bool)> = BTreeMap::new();
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
            if Instant::now() > deadline {
                out.timed_out = true;
            }
            files += 1;
            if files > limits.files || bytes > limits.bytes {
                out.truncated = true;
            }
            if out.timed_out || out.truncated {
                break 'logs;
            }
            if modified_ms(&path).is_some_and(|m| m.saturating_add(MTIME_SLACK_MS) < from) {
                continue;
            }
            let is_recorded = recorded.contains(stem.as_str());
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
            let (mut pending, claims, mut malformed) = parse_session(&text, &stem, ids, false);
            if claims.is_empty() && !is_recorded {
                continue;
            }
            // The subagents of a session are in its own folder, which is not followed if it is a link.
            let session_dir = slug.path().join(&stem);
            let subagent_dir = session_dir.join("subagents");
            let subagent_files = if is_real_dir(&session_dir) && is_real_dir(&subagent_dir) {
                files_in(&subagent_dir)
            } else {
                Vec::new()
            };
            let mut has_subagent_logs = false;
            for sub in subagent_files {
                let sub_path = sub.path();
                if sub_path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                has_subagent_logs = true;
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
                if let Ok(LogFile::Content(sub_text)) = read_text(&sub_path) {
                    bytes += sub_text.len() as u64;
                    let (more, _, bad) = parse_session(&sub_text, &stem, ids, true);
                    malformed += bad;
                    for (key, request) in more {
                        pending.entry(key).or_insert(request);
                    }
                }
            }
            if malformed > 0 {
                out.notes.push(format!(
                    "{malformed} malformed Claude Code log line(s) were skipped"
                ));
            }
            let entry = kept.entry(stem.clone()).or_default();
            entry.0.extend(claims);
            entry.1 |= has_subagent_logs;
            for (key, pending) in pending {
                match seen.get_mut(&key) {
                    Some((_, existing)) => {
                        merge_largest(&mut existing.tokens, &pending.tokens);
                        existing.commands.extend(pending.commands);
                        existing.other_tool |= pending.other_tool;
                        existing.any_tool |= pending.any_tool;
                        existing.tool_output_bytes =
                            existing.tool_output_bytes.max(pending.tool_output_bytes);
                    }
                    None => {
                        seen.insert(key, (stem.clone(), pending));
                    }
                }
            }
        }
    }
    let mut by_session: BTreeMap<String, Vec<Request>> = BTreeMap::new();
    for (session, pending) in seen.into_values() {
        let class = classify(&pending.commands, pending.other_tool, pending.any_tool);
        by_session.entry(session).or_default().push(Request {
            ts: pending.ts,
            category: class.category,
            tokens: pending.tokens,
            chained_reporting: class.chained_reporting,
            several_categories: class.several_categories,
            tool_output_bytes: Some(pending.tool_output_bytes),
            subagent: pending.subagent,
        });
    }
    for (id, (claims, has_subagent_logs)) in kept {
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

/// The responses of one session file keyed by message id, the run ids its executed commands name, and how
/// many lines were not valid JSON.
fn parse_session(
    text: &str,
    session: &str,
    ids: &HashSet<&str>,
    subagent_file: bool,
) -> (BTreeMap<String, Pending>, HashSet<String>, usize) {
    let mut pending: BTreeMap<String, Pending> = BTreeMap::new();
    let mut claims = HashSet::new();
    let mut malformed = 0;
    // Which request made the call a result answers; each call's result counts once.
    let mut owners: HashMap<String, String> = HashMap::new();
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("\"usage\"") || line.contains("\"tool_result\""))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            malformed += 1;
            continue;
        };
        let subagent = subagent_file || data["isSidechain"].as_bool() == Some(true);
        let message = &data["message"];
        for block in message["content"].as_array().into_iter().flatten() {
            if block["type"] != "tool_result" {
                continue;
            }
            if let Some(key) = block["tool_use_id"]
                .as_str()
                .and_then(|id| owners.remove(id))
                && let Some(request) = pending.get_mut(&key)
            {
                request.tool_output_bytes = request
                    .tool_output_bytes
                    .saturating_add(result_bytes(&block["content"]));
            }
        }
        let usage = &message["usage"];
        if !usage.is_object() {
            continue;
        }
        let (Some(_), Some(ts)) = (
            model_name(&message["model"]),
            timestamp(data.get("timestamp")),
        ) else {
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
            continue;
        }
        let key = message["id"]
            .as_str()
            .or(data["uuid"].as_str())
            .map_or_else(
                || format!("{session}|line {number}"),
                |id| format!("{id}|{}", data["requestId"].as_str().unwrap_or("")),
            );
        let entry = pending.entry(key.clone()).or_insert_with(|| Pending {
            ts,
            subagent,
            ..Pending::default()
        });
        merge_largest(&mut entry.tokens, &tokens);
        for block in message["content"].as_array().into_iter().flatten() {
            if block["type"] != "tool_use" {
                continue;
            }
            entry.any_tool = true;
            if let Some(id) = block["id"].as_str() {
                owners.insert(id.to_string(), key.clone());
            }
            let command = (block["name"] == "Bash")
                .then(|| block["input"]["command"].as_str())
                .flatten();
            match command {
                Some(command) => {
                    let command = truncate(command);
                    if !subagent {
                        claims.extend(updated_runs(command, ids));
                    }
                    entry.commands.push(command.to_string());
                }
                None => entry.other_tool = true,
            }
        }
    }
    (pending, claims, malformed)
}

/// The bytes of text in a tool result, which is a string or a list of text blocks.
pub(crate) fn result_bytes(content: &Value) -> u64 {
    let bytes = match content {
        Value::String(text) => text.len(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .map(str::len)
            .sum(),
        _ => 0,
    };
    bytes as u64
}

/// Whether `path` is a folder itself, not a link to one.
fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

pub(crate) fn truncate(command: &str) -> &str {
    if command.len() <= MAX_COMMAND_BYTES {
        return command;
    }
    let mut end = MAX_COMMAND_BYTES;
    while !command.is_char_boundary(end) {
        end -= 1;
    }
    &command[..end]
}

/// The shell variables an agent has set to a literal so far, by name.
pub(crate) type Variables = HashMap<String, String>;

/// The most distinct variable names kept; an assignment to a new name beyond it is ignored. 256 is far
/// above the handful of variables a real agent command sets, and small enough to bound the memory a
/// session holds against a crafted log that assigns endless names.
pub(crate) const MAX_VARIABLES: usize = 256;

/// The run ids in `ids` that a run command in `command` names; see `run_claims`.
pub(crate) fn updated_runs(command: &str, ids: &HashSet<&str>) -> HashSet<String> {
    run_claims(command, ids, &mut Variables::new())
}

/// The run ids in `ids` that a `jidoka run update <id>` or `jidoka run sync <id>` in `command` names: the
/// commands that report on a run, as opposed to reading it. Only a command that runs `jidoka` counts: `grep "jidoka run update 1 "
/// notes.md`, an `echo` or the text of a here-document do not. An id is digits only. The id may be a variable
/// that an earlier command set to a literal (`R=123 && jidoka run update $R`, `export R=123`); `variables`
/// carries those across the commands of one session, in order. A value that comes from a file or another
/// command is not known here.
pub(crate) fn run_claims(
    command: &str,
    ids: &HashSet<&str>,
    variables: &mut Variables,
) -> HashSet<String> {
    let mut found = HashSet::new();
    for segment in &segments(command) {
        let words = split_words(&segment.text);
        let words = match words.first() {
            Some(&"export") => &words[1..],
            _ => &words[..],
        };
        let assigned = words.iter().take_while(|word| is_assignment(word)).count();
        if assigned == words.len() && !words.is_empty() {
            // Only assignments: the variables hold these values for the commands after them.
            for word in words {
                if let Some((name, value)) = word.split_once('=') {
                    let value = value.trim_matches(['"', '\'']);
                    if value.contains(SUBSTITUTION) {
                        variables.remove(name);
                    } else if variables.len() < MAX_VARIABLES || variables.contains_key(name) {
                        variables.insert(name.to_string(), value.to_string());
                    }
                }
            }
            continue;
        }
        let words = command_words(&segment.text);
        let program = words
            .first()
            .map(|word| word.rsplit('/').next().unwrap_or(word));
        let names_a_run = matches!(
            (words.get(1), words.get(2)),
            (Some(&"run"), Some(&("update" | "sync")))
        );
        if let (Some("jidoka"), true, Some(id)) = (program, names_a_run, words.get(3)) {
            let id = id.trim_end_matches([')', '}', ';']);
            let id = match id.strip_prefix('$') {
                Some(name) => variables
                    .get(name.trim_matches(['{', '}']))
                    .map_or("", String::as_str),
                None => id,
            };
            if ids.contains(id) {
                found.insert(id.to_string());
            }
        }
    }
    found
}

// --- categories -------------------------------------------------------------------------------------

/// Programs that only read or reformat text. In a substitution or after a pipe they support the command they
/// belong to; any other program there (`git apply`, `xargs`, `tee`, a script) does work of its own.
fn is_read_only_helper(segment: &str) -> bool {
    const HELPERS: [&str; 20] = [
        "cat", "tail", "head", "grep", "egrep", "fgrep", "rg", "wc", "sort", "uniq", "cut", "tr",
        "jq", "column", "date", "basename", "dirname", "pwd", "whoami", "realpath",
    ];
    let mut words = command_words(segment);
    let program = words
        .first()
        .map(|word| word.rsplit('/').next().unwrap_or(word));
    if program == Some("git") {
        skip_git_options(&mut words);
    }
    let word = |index: usize| words.get(index).copied().unwrap_or("");
    // `git diff --output=<file>` writes a file.
    if words.iter().any(|word| word.starts_with("--output")) {
        return false;
    }
    program.is_some_and(|program| HELPERS.contains(&program))
        || matches!(
            (program, word(1), word(2)),
            (
                Some("git"),
                "rev-parse" | "log" | "status" | "diff" | "show",
                _
            ) | (Some("git"), "branch", "--show-current")
                | (Some("gh"), "pr" | "issue" | "repo", "view")
        )
}

/// Removes the options that come before the subcommand of a git command: `git -C <path> <subcommand>`.
fn skip_git_options(words: &mut Vec<&str>) {
    let skipped = words[1..]
        .chunks(2)
        .take_while(|pair| pair[0] == "-C" || pair[0] == "-c")
        .count();
    words.drain(1..(1 + 2 * skipped).min(words.len()));
}

/// What one command of a shell line stands for.
enum Hit {
    Category(Category),
    Reporting,
    /// `cd`, `echo` and the like: not a command that did the work.
    Neutral,
    Other,
}

/// How a request was classified.
pub(crate) struct Classification {
    pub(crate) category: Category,
    pub(crate) chained_reporting: bool,
    pub(crate) several_categories: bool,
}

/// The category of a request from its Bash commands, whether it called any other tool, and whether it
/// called any tool at all.
pub(crate) fn classify(commands: &[String], other_tool: bool, any_tool: bool) -> Classification {
    if !any_tool {
        return Classification {
            category: Category::TextOnly,
            chained_reporting: false,
            several_categories: false,
        };
    }
    let mut categories: BTreeSet<Category> = BTreeSet::new();
    let (mut reporting, mut other) = (false, other_tool);
    for command in commands {
        for segment in segments(command) {
            match hit(&segment.text) {
                Hit::Category(category) => {
                    categories.insert(category);
                }
                Hit::Reporting => reporting = true,
                Hit::Neutral => {}
                // A read-only helper, such as `cat file` in a substitution or `tail -1` after a pipe, is
                // not work of its own. Anything else that runs there is.
                Hit::Other if segment.plumbing && is_read_only_helper(&segment.text) => {}
                Hit::Other => other = true,
            }
        }
    }
    let category = if let Some(first) = categories.first() {
        *first
    } else if reporting && !other {
        Category::StandaloneReporting
    } else {
        Category::ReadingEditingAnalysis
    };
    Classification {
        category,
        chained_reporting: reporting && category != Category::StandaloneReporting,
        several_categories: categories.len() > 1,
    }
}

/// One command of a shell line.
struct Segment {
    text: String,
    /// It only supports another command: it stands inside a command substitution, or after a `|` where it
    /// filters the output of the command before it. It still runs, but it is not the line's own work.
    plumbing: bool,
}

/// What a command substitution is replaced by in the text of its command, so that it stays one word.
const SUBSTITUTION: char = '\u{1}';

/// How deep command substitutions are followed. Deeper text is dropped, which bounds the work.
const MAX_SUBSTITUTION_DEPTH: usize = 4;

/// Whether `chars[i]` starts a command substitution: `$(` (but not the arithmetic `$((`) or a backtick.
fn substitution_start(chars: &[char], i: usize) -> bool {
    match chars[i] {
        '`' => true,
        '$' => chars.get(i + 1) == Some(&'(') && chars.get(i + 2) != Some(&'('),
        _ => false,
    }
}

/// The delimiter of the here-document that starts with the `<<` at `chars[i]` (`<<EOF`, `<<-EOF`,
/// `<<'EOF'`, `<<\EOF`), and the index after its word. Arithmetic such as `$((1<<2))` has no delimiter
/// word, so it starts no here-document.
fn heredoc_delimiter(chars: &[char], i: usize) -> (Option<String>, usize) {
    let mut j = i + 2;
    while chars.get(j).is_some_and(|c| matches!(c, '-' | ' ' | '\t')) {
        j += 1;
    }
    let start = j;
    while chars
        .get(j)
        .is_some_and(|c| !c.is_whitespace() && !";|&<>()".contains(*c))
    {
        j += 1;
    }
    let word: String = chars[start..j].iter().collect();
    let word = word.trim_matches(['\'', '"']).trim_start_matches('\\');
    let valid = word
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    (valid.then(|| word.to_string()), j)
}

/// The index of the last character of a here-document body: the lines after the line end at `chars[newline]`,
/// up to and including the delimiter line.
fn skip_heredoc_body(chars: &[char], newline: usize, delimiter: &str) -> usize {
    let mut line_start = newline + 1;
    while line_start < chars.len() {
        let line_end = chars[line_start..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(chars.len(), |p| line_start + p);
        let line: String = chars[line_start..line_end].iter().collect();
        line_start = line_end + 1;
        if line.trim() == delimiter {
            break;
        }
    }
    line_start.saturating_sub(1)
}

/// The text inside the substitution that starts at `chars[i]`, and the index of its last character. An
/// unterminated substitution runs to the end.
fn substitution_at(chars: &[char], i: usize) -> (String, usize) {
    if chars[i] == '`' {
        let mut j = i + 1;
        while j < chars.len() && chars[j] != '`' {
            j += if chars[j] == '\\' { 2 } else { 1 };
        }
        let end = j.min(chars.len());
        return (chars[i + 1..end].iter().collect(), j.min(chars.len() - 1));
    }
    let (mut depth, mut quote, mut j) = (1usize, None::<char>, i + 2);
    let mut heredoc: Option<String> = None;
    while j < chars.len() {
        let c = chars[j];
        match quote {
            Some(open) => {
                if c == '\\' && open == '"' {
                    j += 1;
                } else if c == open {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                // A here-string, `<<<`, has no delimiter.
                '<' if chars.get(j + 1) == Some(&'<') && chars.get(j + 2) == Some(&'<') => j += 2,
                // The text of a here-document, such as a commit message, may hold quotes and parentheses.
                '<' if chars.get(j + 1) == Some(&'<') => {
                    let (delimiter, after) = heredoc_delimiter(chars, j);
                    heredoc = delimiter;
                    j = after - 1;
                }
                '\n' => {
                    if let Some(delimiter) = heredoc.take() {
                        j = skip_heredoc_body(chars, j, &delimiter);
                    }
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return (chars[i + 2..j].iter().collect(), j);
                    }
                }
                _ => {}
            },
        }
        j += 1;
    }
    (chars[i + 2..].iter().collect(), chars.len() - 1)
}

/// Replaces the substitution that starts at `chars[i]` by one word in `current`, adds the commands inside it
/// to `nested` (down to `MAX_SUBSTITUTION_DEPTH`), and returns the index of its last character.
fn take_substitution(
    chars: &[char],
    i: usize,
    depth: usize,
    current: &mut String,
    nested: &mut Vec<Segment>,
) -> usize {
    let (inner, end) = substitution_at(chars, i);
    current.push(SUBSTITUTION);
    if depth < MAX_SUBSTITUTION_DEPTH {
        nested.extend(segments_at(&inner, depth + 1));
    }
    end
}

/// The commands of a shell line, split at `&&`, `||`, `;`, `|` and line ends. Text inside quotes stays in
/// its command; comments, here-strings and the lines of a here-document are text, not commands. A
/// here-document delimiter may be bare, quoted or backslash-escaped. A command substitution is one word of
/// its command, and the commands inside it follow as nested segments.
fn segments(command: &str) -> Vec<Segment> {
    segments_at(command, 0)
}

fn segments_at(command: &str, depth: usize) -> Vec<Segment> {
    let mut nested: Vec<Segment> = Vec::new();
    let chars: Vec<char> = command.chars().collect();
    let mut out: Vec<Segment> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut heredoc: Option<String> = None;
    // Whether the command being read follows a `|`.
    let mut piped = false;
    let mut flush = |current: &mut String, piped: bool| {
        let part = current.trim();
        if !part.is_empty() {
            out.push(Segment {
                text: part.to_string(),
                plumbing: depth > 0 || piped,
            });
        }
        current.clear();
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if quote == Some('"') && substitution_start(&chars, i) {
            i = take_substitution(&chars, i, depth, &mut current, &mut nested);
        } else if let Some(open) = quote {
            current.push(c);
            if c == '\\' && open == '"' {
                if let Some(escaped) = next {
                    current.push(escaped);
                    i += 1;
                }
            } else if c == open {
                quote = None;
            }
        } else {
            match c {
                '\\' => {
                    current.push(c);
                    if let Some(escaped) = next {
                        current.push(escaped);
                        i += 1;
                    }
                }
                '\'' | '"' => {
                    quote = Some(c);
                    current.push(c);
                }
                '$' | '`' if substitution_start(&chars, i) => {
                    i = take_substitution(&chars, i, depth, &mut current, &mut nested);
                }
                '#' if current.chars().next_back().is_none_or(char::is_whitespace) => {
                    // A comment runs to the end of the line.
                    while chars.get(i + 1).is_some_and(|c| *c != '\n') {
                        i += 1;
                    }
                }
                '<' if next == Some('<') && chars.get(i + 2) == Some(&'<') => {
                    // A here-string, `<<<`, has no delimiter.
                    current.push_str("<<<");
                    i += 2;
                }
                '<' if next == Some('<') => {
                    let (delimiter, after) = heredoc_delimiter(&chars, i);
                    heredoc = delimiter;
                    current.push_str("<<");
                    i = after - 1;
                }
                '\n' => {
                    flush(&mut current, piped);
                    piped = false;
                    if let Some(delimiter) = heredoc.take() {
                        i = skip_heredoc_body(&chars, i, &delimiter);
                    }
                }
                ';' | '|' => {
                    flush(&mut current, piped);
                    // `||` is a separator like `;`; a single `|` pipes into the next command.
                    piped = c == '|' && next != Some('|');
                    if c == '|' && next == Some('|') {
                        i += 1;
                    }
                }
                '&' if next == Some('&') => {
                    flush(&mut current, piped);
                    piped = false;
                    i += 1;
                }
                _ => current.push(c),
            }
        }
        i += 1;
    }
    flush(&mut current, piped);
    out.extend(nested);
    out
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The most words of a command that are looked at. The program and its subcommand are at the start, so a
/// hostile command of thousands of words costs no more than a normal one.
const MAX_COMMAND_WORDS: usize = 32;

/// The words of a command, split at whitespace outside quotes; the quotes stay in the words. At most
/// `MAX_COMMAND_WORDS` are read.
fn split_words(segment: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let (mut start, mut quote) = (None::<usize>, None::<char>);
    for (at, c) in segment.char_indices() {
        match quote {
            Some(open) => {
                if c == open {
                    quote = None;
                }
            }
            None if c.is_whitespace() => {
                if let Some(from) = start.take() {
                    words.push(&segment[from..at]);
                    if words.len() >= MAX_COMMAND_WORDS {
                        return words;
                    }
                }
            }
            None => {
                start.get_or_insert(at);
                if c == '\'' || c == '"' {
                    quote = Some(c);
                }
            }
        }
    }
    if let Some(from) = start {
        words.push(&segment[from..]);
    }
    words
}

/// The words of one command without quotes, env assignments and the words that only wrap it: `sudo`,
/// `time`, `command`, `exec`, `nohup`, the shell keywords `if`, `then`, `do`, `else`, `elif`, `while` and
/// `until`, `!`, an opening parenthesis or brace, and `env` and `timeout` with their options (and, for
/// `timeout`, the duration).
fn command_words(segment: &str) -> Vec<&str> {
    let words: Vec<&str> = split_words(segment)
        .into_iter()
        .map(|word| {
            word.trim_matches(['"', '\''])
                .trim_start_matches(['(', '{'])
        })
        .filter(|word| !word.is_empty())
        .collect();
    const WRAPPERS: [&str; 13] = [
        "sudo", "time", "command", "exec", "nohup", "if", "then", "do", "else", "elif", "while",
        "until", "!",
    ];
    let mut start = 0;
    while let Some(word) = words.get(start) {
        if is_assignment(word) || WRAPPERS.contains(word) {
            start += 1;
        } else if *word == "env" || *word == "timeout" {
            // `env [options] [NAME=value]... <command>` and `timeout [options] <duration> <command>`
            start += 1;
            while let Some(option) = words.get(start).filter(|w| w.starts_with('-')) {
                // Options that take their value as the next word.
                start +=
                    if ["-u", "-s", "-k", "--signal", "--kill-after", "--unset"].contains(option) {
                        2
                    } else {
                        1
                    };
            }
            if *word == "timeout" {
                start += 1;
            }
        } else {
            break;
        }
    }
    words[start.min(words.len())..].to_vec()
}

fn hit(segment: &str) -> Hit {
    let mut words = command_words(segment);
    let Some(program) = words
        .first()
        .map(|word| word.rsplit('/').next().unwrap_or(word))
    else {
        return Hit::Neutral;
    };
    if program == "git" {
        skip_git_options(&mut words);
    }
    let word = |index: usize| words.get(index).copied().unwrap_or("");
    let has = |text: &str| words.iter().any(|w| w.contains(text));
    let category = match (program, word(1), word(2)) {
        ("cd" | "pushd" | "popd" | "echo" | "export" | "set" | "true" | ":" | "printf", _, _) => {
            return Hit::Neutral;
        }
        ("git", "worktree", "list") => Category::ContinuationCheck,
        ("git", "worktree" | "clone" | "symbolic-ref", _) => Category::Workspace,
        ("git", "commit" | "add", _) => Category::Commit,
        ("git", "push", _) => Category::PrCreationAndPush,
        ("git", "fetch" | "merge" | "merge-base" | "pull", _) => Category::PrSynchronization,
        ("git", "rev-parse", _) if has("--git-common-dir") => Category::ContinuationCheck,
        ("gh", "pr", "create" | "edit") => Category::PrCreationAndPush,
        ("gh", "pr", "checks") | ("gh", "run", "watch" | "view" | "list") => Category::CiWatch,
        ("gh", "pr", "view") if has("mergeStateStatus") => Category::PrSynchronization,
        ("gh", "api", _) if has("actions/runs") || has("check-runs") => Category::CiWatch,
        ("az", "repos", "pr") if matches!(word(3), "create" | "update") => {
            Category::PrCreationAndPush
        }
        ("az", "repos", "pr") if word(3) == "policy" && word(4) == "list" => Category::CiWatch,
        ("az", "pipelines", "runs") => Category::CiWatch,
        // `pr wait` reads checks with `--resource pullRequestStatuses` or `pullRequestIterationStatuses`.
        ("az", "devops", "invoke")
            if words
                .iter()
                .any(|w| matches!(*w, "pullRequestStatuses" | "pullRequestIterationStatuses")) =>
        {
            Category::CiWatch
        }
        ("jidoka", "run", "start") => Category::Workspace,
        ("jidoka", "run", "update") => return Hit::Reporting,
        ("jidoka", "run", "inspect" | "check") => Category::ContinuationCheck,
        ("cargo", "test" | "fmt" | "clippy" | "build" | "check" | "nextest", _) => {
            Category::Verification
        }
        ("rustfmt" | "pytest", _, _) => Category::Verification,
        ("npm" | "pnpm" | "yarn" | "bun", "test" | "lint" | "build", _) => Category::Verification,
        ("npm" | "pnpm" | "yarn" | "bun", "run", "test" | "lint" | "build" | "check") => {
            Category::Verification
        }
        ("go", "test" | "vet" | "build", _) => Category::Verification,
        ("dotnet", "test" | "build", _) => Category::Verification,
        ("make", "test" | "check" | "lint", _) => Category::Verification,
        ("sh" | "bash" | "zsh", script, _) if script.ends_with("verify.sh") => {
            Category::Verification
        }
        (script, _, _) if script.ends_with("verify.sh") => Category::Verification,
        _ => return Hit::Other,
    };
    Hit::Category(category)
}
