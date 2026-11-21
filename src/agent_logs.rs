//! Readers for the session logs of Claude Code, Codex and Copilot, for Token usage and for the request
//! analysis alike. A response keeps only its model, reasoning effort (Claude Code and Codex record it), token
//! counts, timestamp and the Bash commands of its tool calls, which the request analysis classifies and
//! drops; a session keeps the names and times of the skills the agent used and which run ids its executed
//! Jidoka commands name (the Session claim in the glossary). No conversation text, prompt, tool output or
//! credential leaves this module. Claude Code logs also give the moments of pause markers (usage limit,
//! compaction) and of activity, as timestamps only.
//!
//! Formats (see the README for what is validated): Claude Code transcripts and their subagent transcripts,
//! Codex rollout files, and Copilot `events.jsonl` files. Claude Code is read by one adapter that both
//! reports share; Codex and Copilot move behind the same interface next.

use crate::{
    install::agent_home,
    request_logs::{Variables, is_real_dir, result_bytes, run_claims, truncate},
    run::Run,
    usage::{Entry, PauseKind, Tokens, overlaps, parse_timestamp},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{Instant, UNIX_EPOCH},
};

/// The largest log read: real session logs are a few megabytes, so a bigger one is skipped.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// The longest model name or session id kept from a log: real ones are far shorter, so a longer one is junk.
const MAX_NAME_LEN: usize = 100;
pub(crate) const MTIME_SLACK_MS: u64 = 5 * 60 * 1000;
/// Limits on one pass over the logs, so a huge or hostile log folder cannot exhaust memory or time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadLimits {
    pub(crate) files: usize,
    pub(crate) bytes: u64,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            files: 20_000,
            bytes: 512 * 1024 * 1024,
        }
    }
}

/// Where each agent keeps its session logs.
#[derive(Clone, Debug)]
pub(crate) struct Roots {
    pub(crate) claude: PathBuf,
    pub(crate) codex: PathBuf,
    pub(crate) copilot: PathBuf,
}

impl Roots {
    /// The agents' usual folders under `home`, honoring `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `COPILOT_HOME` as install does.
    pub(crate) fn from_home(
        home: &Path,
        var: &dyn Fn(&str) -> Option<OsString>,
    ) -> io::Result<Self> {
        Ok(Self {
            claude: agent_home("claude", home, var)?.join("projects"),
            codex: agent_home("codex", home, var)?.join("sessions"),
            copilot: agent_home("copilot", home, var)?.join("session-state"),
        })
    }
}

/// A session that names a run or was recorded for one, with its de-duplicated entries.
#[derive(Debug)]
pub(crate) struct RunSession {
    pub(crate) id: String,
    /// The run ids the log names.
    pub(crate) mentions: HashSet<String>,
    pub(crate) entries: Vec<Entry>,
    /// Pause markers in the log: what and when, never text. Claude Code only.
    pub(crate) pauses: Vec<(PauseKind, u64)>,
    /// When the log shows the agent or the user working, sorted. Claude Code only.
    pub(crate) activity: Vec<u64>,
    pub(crate) attention: Vec<(u64, crate::attention::Signal)>,
    /// The skills the agent used, with when, sorted. Only the skill's name is kept.
    pub(crate) skills: Vec<(u64, String)>,
}

/// Something that limits what a session's usage covers.
#[derive(Debug)]
pub(crate) struct Note {
    /// The session it is about; `None` when it concerns the reading as a whole.
    pub(crate) session: Option<String>,
    pub(crate) text: String,
}

#[derive(Debug, Default)]
pub(crate) struct ReadResult {
    pub(crate) sessions: Vec<RunSession>,
    pub(crate) notes: Vec<Note>,
    /// How many of the three log folders exist.
    pub(crate) roots_found: usize,
    pub(crate) timed_out: bool,
    /// Reading stopped at a limit on the number or size of the logs.
    pub(crate) truncated: bool,
}

// --- shared pieces ----------------------------------------------------------------------------------

/// A model name that is safe to store and print: a log is written by another program.
pub(crate) fn model_name(value: &Value) -> Option<String> {
    let name = value.as_str()?;
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphanumeric()
        && name.len() <= MAX_NAME_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:/@+-".contains(c)))
    .then(|| name.to_string())
}

/// Longest effort text kept. The known efforts (`low`, `medium`, `high`, `xhigh`, `max`, `ultra`) are short.
const MAX_EFFORT_LEN: usize = 16;

/// A reasoning effort that is safe to store and print: a log is written by another program. Claude Code
/// writes it as `effort` (and `perTurnEffort`, the same value) on each response.
fn effort_name(value: &Value) -> Option<String> {
    let name = value.as_str()?;
    (!name.is_empty()
        && name.len() <= MAX_EFFORT_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    .then(|| name.to_string())
}

/// A token count, or `None` when the log holds no usable one.
pub(crate) fn count(value: Option<&Value>) -> Option<u64> {
    value?.as_u64().filter(|n| *n < (1 << 53))
}

pub(crate) fn timestamp(value: Option<&Value>) -> Option<u64> {
    parse_timestamp(value?.as_str()?)
}

pub(crate) fn modified_ms(path: &Path) -> Option<u64> {
    let modified = fs::symlink_metadata(path).ok()?.modified().ok()?;
    u64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

pub(crate) enum LogFile {
    Content(String),
    /// Not a regular file, so not followed.
    Skipped,
    TooBig,
}

/// The text of a regular log file; a link or special file is not followed.
pub(crate) fn read_text(path: &Path) -> io::Result<LogFile> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Ok(LogFile::Skipped);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Ok(LogFile::TooBig);
    }
    Ok(LogFile::Content(
        String::from_utf8_lossy(&fs::read(path)?).into_owned(),
    ))
}

/// The run ids a log names *in a command on the run*: a known run id right after `run update`,
/// `run inspect`, `run stats`, `run check`, `run sync`, `pr open`, `pr sync` or `pr wait`, as `jidoka run update <id>`
/// is written. The command word must start the text or follow whitespace, so `xpr open <id>` does not count.
/// Run ids also appear in tool output and in files an agent reads (a session that listed every run would
/// otherwise claim them all), so a bare number does not count. A run id is digits only, so a number inside
/// a longer number does not match.
pub(crate) fn mentioned_runs(text: &str, ids: &HashSet<&str>) -> HashSet<String> {
    const COMMANDS: [&str; 8] = [
        "run update",
        "run inspect",
        "run stats",
        "run check",
        "run sync",
        "pr open",
        "pr sync",
        "pr wait",
    ];
    let bytes = text.as_bytes();
    let mut found = HashSet::new();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if !(8..=22).contains(&(at - start)) || !ids.contains(&text[start..at]) {
            continue;
        }
        // What comes just before the number: the command, then spaces and an optional quote.
        let before = text.as_bytes()[..start]
            .iter()
            .rev()
            .take(24)
            .copied()
            .collect::<Vec<u8>>();
        let before: String = before.into_iter().rev().map(char::from).collect();
        let before = before.trim_end_matches(['"', '\'', '`', ' ']);
        let command_starts_a_word = COMMANDS.iter().any(|command| {
            before.strip_suffix(command).is_some_and(|rest| {
                rest.chars()
                    .next_back()
                    .is_none_or(|c| c.is_ascii_whitespace())
            })
        });
        if command_starts_a_word {
            found.insert(text[start..at].to_string());
        }
    }
    found
}

fn sorted_dir(path: &Path) -> Vec<fs::DirEntry> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(path)
        .map(|dir| dir.flatten().collect())
        .unwrap_or_default();
    entries.sort_by_key(fs::DirEntry::file_name);
    entries
}

/// The folders inside `path`. A link to a folder is not followed, so a link cannot lead out of the log folder.
pub(crate) fn sub_dirs(path: &Path) -> Vec<fs::DirEntry> {
    let mut entries = sorted_dir(path);
    entries.retain(|e| e.file_type().is_ok_and(|t| t.is_dir()));
    entries
}

/// The regular files inside `path`; links are not followed.
pub(crate) fn files_in(path: &Path) -> Vec<fs::DirEntry> {
    let mut entries = sorted_dir(path);
    entries.retain(|e| e.file_type().is_ok_and(|t| t.is_file()));
    entries
}

/// A response is often written more than once, sometimes with growing counts: the largest of each field wins.
pub(crate) fn merge_largest(into: &mut Tokens, other: &Tokens) {
    let max = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    into.input = max(into.input, other.input);
    into.cache_read = max(into.cache_read, other.cache_read);
    into.cache_write = max(into.cache_write, other.cache_write);
    into.output = max(into.output, other.output);
    into.reasoning = max(into.reasoning, other.reasoning);
}

/// What one log file gave. Lines that looked like usage but could not be used are counted, so the
/// coverage can say so instead of silently looking complete.
#[derive(Default)]
struct LogUsage {
    entries: Vec<(String, Entry)>,
    /// Lines that were not valid JSON.
    malformed: usize,
    /// Valid lines that held no usable model, time or counts.
    unusable: usize,
    /// Entries that carry no response id, so a repeat of them cannot be told from a new call.
    unidentified: usize,
    pauses: Vec<(PauseKind, u64)>,
    activity: Vec<u64>,
    attention: Vec<(u64, crate::attention::Signal)>,
    skills: Vec<(u64, String)>,
}

impl LogUsage {
    /// Adds an entry under its response id, or, with none, under its line so it counts once as written.
    fn push(&mut self, id: Option<String>, line: usize, prefix: &str, entry: Entry) {
        let key = match id {
            Some(id) => format!("{prefix}|{id}"),
            None => {
                self.unidentified += 1;
                format!("{prefix}|{}|line {line}", entry.session)
            }
        };
        self.entries.push((key, entry));
    }

    fn absorb(&mut self, other: LogUsage) {
        self.malformed += other.malformed;
        self.unusable += other.unusable;
        self.unidentified += other.unidentified;
        self.entries.extend(other.entries);
        self.pauses.extend(other.pauses);
        self.activity.extend(other.activity);
        self.attention.extend(other.attention);
        self.skills.extend(other.skills);
    }
}

/// The longest skill name kept: real ones are far shorter, so a longer one is junk.
const MAX_SKILL_NAME_LEN: usize = 64;

/// Whether `c` can be part of a skill name: letters, digits and `-_.:` (`:` joins a plugin and its skill).
fn is_skill_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.:".contains(c)
}

/// A skill name as the logs write it. Anything else is not a skill name, so no path or text slips into a report.
fn skill_name(raw: &str) -> Option<String> {
    let plain = raw.len() <= MAX_SKILL_NAME_LEN
        && raw.chars().all(is_skill_char)
        && raw.chars().any(|c| c.is_ascii_alphanumeric());
    plain.then(|| raw.to_string())
}

/// The programs that read a file to the agent. A command that only lists, searches or edits a skill's
/// file (`ls`, `rg`, `git diff`, `apply_patch`) does not load the skill, so it does not count.
const FILE_READERS: [&str; 9] = [
    "cat",
    "sed",
    "head",
    "tail",
    "less",
    "more",
    "bat",
    "nl",
    "Get-Content",
];

/// The names of the skills a shell command reads: each `<name>/SKILL.md` (or `<name>\SKILL.md`) in a
/// command that runs a file reader. That is how Codex loads a skill, as it has no tool for it. A command
/// that also lists or searches skill files is still counted when it reads one; the names it lists are
/// then counted too, which is a heuristic.
fn skills_read_by(command: &str) -> Vec<String> {
    let reads = command
        .split(|c: char| c.is_whitespace() || "\"'`;&|(){}".contains(c))
        .any(|word| FILE_READERS.contains(&word.rsplit('/').next().unwrap_or(word)));
    if !reads || command.contains("*** ") {
        return Vec::new();
    }
    command
        .match_indices("SKILL.md")
        .filter_map(|(end, _)| {
            // Arguments are JSON text, so a Windows separator may be doubled.
            let path = &command[..end];
            let before = path.trim_end_matches(['/', '\\']);
            if before.len() == path.len() {
                return None;
            }
            let start = before
                .rfind(|c: char| !is_skill_char(c))
                .map_or(0, |at| at + 1);
            skill_name(&before[start..])
        })
        .collect()
}

// --- Claude Code ------------------------------------------------------------------------------------

/// One response of a session, reduced to what usage and the request analysis need.
#[derive(Clone, Debug)]
pub(crate) struct Response {
    pub(crate) session: String,
    /// Milliseconds since the Unix epoch.
    pub(crate) ts: u64,
    pub(crate) model: String,
    /// The reasoning effort the log recorded for the call, when it recorded one.
    pub(crate) effort: Option<String>,
    pub(crate) tokens: Tokens,
    /// Whether a subagent made the call.
    pub(crate) subagent: bool,
    /// The Bash commands of its tool calls, for the request analysis to classify. Nothing else of a tool
    /// call is kept.
    pub(crate) commands: Vec<String>,
    /// It called a tool that is not Bash.
    pub(crate) other_tool: bool,
    pub(crate) any_tool: bool,
    /// The bytes of text its tool calls returned; unknown when the log does not say.
    pub(crate) tool_output_bytes: Option<u64>,
}

/// A session that executed a Jidoka command naming a run, or was recorded for one.
#[derive(Debug)]
pub(crate) struct LogSession {
    pub(crate) id: String,
    /// Whether the session has subagent logs. Without them its figures cover the coordinator only.
    pub(crate) has_subagent_logs: bool,
    /// The run ids an executed Jidoka command in this session names.
    pub(crate) claims: HashSet<String>,
    /// De-duplicated, sorted by time.
    pub(crate) responses: Vec<Response>,
    pub(crate) pauses: Vec<(PauseKind, u64)>,
    pub(crate) activity: Vec<u64>,
    pub(crate) attention: Vec<(u64, crate::attention::Signal)>,
    pub(crate) skills: Vec<(u64, String)>,
}

/// What one adapter read.
#[derive(Debug, Default)]
pub(crate) struct LogRead {
    pub(crate) sessions: Vec<LogSession>,
    pub(crate) notes: Vec<Note>,
    /// Whether the agent's log folder exists.
    pub(crate) root_found: bool,
    pub(crate) timed_out: bool,
    pub(crate) truncated: bool,
}

/// The runs whose sessions one reading looks for, and since when: the target and the runs that overlap its
/// time, because only they can contest its sessions. Usage and the request analysis both read the logs
/// with the same window.
pub(crate) struct RunWindow<'a> {
    pub(crate) tracked: Vec<&'a Run>,
    pub(crate) ids: HashSet<&'a str>,
    /// The session ids the tracked runs recorded.
    pub(crate) recorded: HashSet<&'a str>,
    /// The earliest start of a tracked run, in milliseconds.
    pub(crate) from: u64,
}

impl<'a> RunWindow<'a> {
    /// `first` is the start of `target`'s timeline.
    pub(crate) fn new(target: &'a Run, runs: &'a [Run], first: u64) -> Self {
        let tracked: Vec<&Run> = runs
            .iter()
            .filter(|run| {
                run.id == target.id || (!run.timeline.is_empty() && overlaps(target, run))
            })
            .collect();
        let ids = tracked.iter().map(|run| run.id.as_str()).collect();
        let recorded = tracked
            .iter()
            .flat_map(|run| run.sessions.iter().map(String::as_str))
            .collect();
        let from = tracked
            .iter()
            .filter_map(|run| run.timeline.first())
            .map(|mark| mark.at)
            .min()
            .unwrap_or(first);
        Self {
            tracked,
            ids,
            recorded,
            from,
        }
    }
}

/// What one Claude Code log file gave. Lines that looked like a response but could not be used are
/// counted, so the notes can say so instead of silently looking complete.
#[derive(Default)]
struct ClaudeLog {
    /// Responses by key; Claude Code writes one per content block, so a response counts once.
    responses: BTreeMap<String, Response>,
    claims: HashSet<String>,
    /// Lines that were not valid JSON.
    malformed: usize,
    /// Valid lines that held no usable model, time or counts.
    unusable: usize,
    /// Responses that carry no id, so a repeat of them cannot be told from a new call.
    unidentified: usize,
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
            parsed.malformed += 1;
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
            parsed.unusable += 1;
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
            parsed.unusable += 1;
            continue;
        }
        let key = match message["id"].as_str().or(data["uuid"].as_str()) {
            Some(id) => format!("claude|{id}|{}", data["requestId"].as_str().unwrap_or("")),
            None => {
                parsed.unidentified += 1;
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

/// Whether a log's own file or folder name is safe to keep as a session id: a plain one.
fn plain_id(id: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
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
                        parsed.malformed += more.malformed;
                        parsed.unusable += more.unusable;
                        parsed.unidentified += more.unidentified;
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
            let mut note = |text: String| {
                state.notes.push(Note {
                    session: Some(stem.clone()),
                    text,
                });
            };
            if parsed.malformed > 0 {
                note(format!(
                    "{} malformed Claude Code log line(s) were skipped",
                    parsed.malformed
                ));
            }
            if parsed.unusable > 0 {
                note(format!(
                    "{} Claude Code usage line(s) had no usable model, time or count and were skipped",
                    parsed.unusable
                ));
            }
            if parsed.unidentified > 0 {
                note(format!(
                    "{} Claude Code usage entr(ies) have no response id, so a repeat of them cannot be detected",
                    parsed.unidentified
                ));
            }
            if parsed.responses.is_empty() {
                note(
                    "a Claude Code session log held no usage entries in a recognised format".into(),
                );
            }
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

// --- Codex ------------------------------------------------------------------------------------------

/// A rollout seen on the first pass: its ids and where it is, not its text, so unclaimed rollouts
/// cost no memory.
struct Rollout {
    path: PathBuf,
    id: String,
    root: String,
    subagent: bool,
    mentions: HashSet<String>,
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

/// Reasoning tokens are part of output; see `codex_tokens` for how the counts are split.
fn parse_codex(text: &str, session: &str, subagent: bool) -> LogUsage {
    let mut parsed = LogUsage::default();
    let mut by_turn: BTreeMap<String, TurnSettings> = BTreeMap::new();
    let mut last: Option<TurnSettings> = None;
    for (number, line) in text.lines().enumerate() {
        let is_context = line.contains("\"turn_context\"");
        if !is_context && !line.contains("\"token_usage_record\"") {
            continue;
        }
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            parsed.malformed += 1;
            continue;
        };
        let payload = &data["payload"];
        if is_context && data["type"] == "turn_context" {
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
            continue;
        }
        if data["type"] != "token_usage_record" {
            continue;
        }
        let usage = &payload["usage"];
        let (Some(ts), true) = (timestamp(data.get("timestamp")), usage.is_object()) else {
            parsed.unusable += 1;
            continue;
        };
        // The turn's own settings, or the latest ones when the record names no known turn.
        let settings = payload["turn_id"]
            .as_str()
            .and_then(|t| by_turn.get(t))
            .or(last.as_ref());
        let Some(settings) = settings.filter(|_| has_token_counts(usage)) else {
            parsed.unusable += 1;
            continue;
        };
        let tokens = codex_tokens(usage);
        parsed.push(
            payload["response_id"].as_str().map(str::to_string),
            number,
            "codex",
            Entry {
                session: session.to_string(),
                ts,
                model: settings.model.clone(),
                tokens,
                subagent,
                effort: settings.effort.clone(),
            },
        );
    }
    parsed
}

/// The skills a Codex rollout shows used: tool calls whose command reads a `<name>/SKILL.md`. Tool output
/// and the session's own instructions mention skill files too, so only the calls are looked at.
fn codex_skills(text: &str, parsed: &mut LogUsage) {
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

fn read_codex(root: &Path, pass: &mut ReadPass, out: &mut ReadResult) {
    let earliest_day = civil_day(pass.from.saturating_sub(DAY_MS));
    let mut rollouts: Vec<Rollout> = Vec::new();
    for year in sub_dirs(root) {
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
                    let fname = file.file_name().to_string_lossy().into_owned();
                    if !(fname.starts_with("rollout-") && fname.ends_with(".jsonl")) {
                        continue;
                    }
                    if pass.over_limit(out) {
                        return;
                    }
                    if pass.stale(&path) {
                        continue;
                    }
                    let Some(text) = pass.load(&path, &fname, out) else {
                        continue;
                    };
                    let Some((id, root, subagent)) = codex_meta(&text) else {
                        continue;
                    };
                    let mentions = if subagent {
                        HashSet::new()
                    } else {
                        crate::codex_requests::codex_claims(&text, pass.ids)
                    };
                    rollouts.push(Rollout {
                        path,
                        id,
                        root,
                        subagent,
                        mentions,
                    });
                }
            }
        }
    }
    // A subagent's rollout names the run only through its root session, so the roots decide.
    let claimed_roots: HashSet<String> = rollouts
        .iter()
        .filter(|r| {
            !r.mentions.is_empty()
                || pass.recorded.contains(r.id.as_str())
                || pass.recorded.contains(r.root.as_str())
        })
        .map(|r| r.root.clone())
        .collect();
    let mut by_root: BTreeMap<String, (HashSet<String>, LogUsage)> = BTreeMap::new();
    for rollout in rollouts.iter().filter(|r| claimed_roots.contains(&r.root)) {
        if pass.over_limit(out) {
            return;
        }
        // Only a claimed rollout is read again, to parse it.
        let Some(text) = pass.load(&rollout.path, &rollout.root, out) else {
            continue;
        };
        let mut parsed = parse_codex(&text, &rollout.root, rollout.subagent);
        codex_skills(&text, &mut parsed);
        if !rollout.subagent {
            parsed.attention = crate::attention::signals(crate::attention::Agent::Codex, &text);
        }
        let (mentions, all) = by_root.entry(rollout.root.clone()).or_default();
        mentions.extend(rollout.mentions.iter().cloned());
        all.absorb(parsed);
    }
    for (root, (mentions, parsed)) in by_root {
        pass.add(&root, mentions, parsed, "Codex", out);
    }
}

/// `YYYY-MM-DD` for milliseconds since the epoch, to compare with the day folders.
pub(crate) fn civil_day(millis: u64) -> String {
    crate::usage::format_timestamp(millis)[..10].to_string()
}

// --- Copilot ----------------------------------------------------------------------------------------

/// The tokens of a Copilot `usage` object, or `None` when it reports neither input nor output.
pub(crate) fn copilot_tokens(usage: &Value) -> Option<Tokens> {
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

/// Copilot counts cache reads and writes inside `inputTokens`; both are taken out of input. Reasoning
/// tokens are part of output. Each record also names the effort of its call as `reasoningEffort`.
fn parse_copilot(text: &str, session: &str) -> LogUsage {
    let mut parsed = LogUsage::default();
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("\"session.usage_record\""))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            parsed.malformed += 1;
            continue;
        };
        if data["type"] != "session.usage_record" {
            continue;
        }
        let usage = &data["data"]["usage"];
        let (Some(model), Some(ts)) = (
            model_name(&usage["model"]),
            timestamp(data.get("timestamp")),
        ) else {
            parsed.unusable += 1;
            continue;
        };
        let Some(tokens) = copilot_tokens(usage) else {
            parsed.unusable += 1;
            continue;
        };
        parsed.push(
            usage["apiCallId"].as_str().map(str::to_string),
            number,
            &format!("copilot|{session}"),
            Entry {
                session: session.to_string(),
                ts,
                model,
                tokens,
                subagent: usage["initiator"] == "sub-agent",
                effort: effort_name(&usage["reasoningEffort"]),
            },
        );
    }
    parsed
}

/// The skills a Copilot log shows used: `skill` tool calls.
fn copilot_skills(text: &str, parsed: &mut LogUsage) {
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
            parsed.skills.push((ts, name));
        }
    }
}

fn read_copilot(root: &Path, pass: &mut ReadPass, out: &mut ReadResult) {
    for dir in sub_dirs(root) {
        let session = dir.file_name().to_string_lossy().into_owned();
        let path = dir.path().join("events.jsonl");
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        if pass.over_limit(out) {
            return;
        }
        if pass.stale(&path) {
            continue;
        }
        let Some(text) = pass.load(&path, &session, out) else {
            continue;
        };
        let mentions = crate::copilot_requests::copilot_claims(&text, pass.ids);
        if mentions.is_empty() && !pass.recorded.contains(session.as_str()) {
            continue;
        }
        let mut parsed = parse_copilot(&text, &session);
        copilot_skills(&text, &mut parsed);
        parsed.attention = crate::attention::signals(crate::attention::Agent::Copilot, &text);
        pass.add(&session, mentions, parsed, "Copilot", out);
    }
}

// --- reading ----------------------------------------------------------------------------------------

/// A session's pause markers and moments of activity.
type Markers = (Vec<(PauseKind, u64)>, Vec<u64>);

/// One pass over the agent logs: what to look for and the limits, plus what the readers have gathered so far.
struct ReadPass<'a> {
    ids: &'a HashSet<&'a str>,
    recorded: &'a HashSet<&'a str>,
    from: u64,
    deadline: Instant,
    limits: ReadLimits,
    files: usize,
    bytes: u64,
    /// Entries by key across every session, so a response that appears twice counts once.
    entries: BTreeMap<String, Entry>,
    /// Per session: the run ids it names.
    sessions: BTreeMap<String, HashSet<String>>,
    /// Per session: its pause markers and moments of activity.
    pauses: BTreeMap<String, Markers>,
    attention: BTreeMap<String, Vec<(u64, crate::attention::Signal)>>,
    /// Per session: the skills it used and when.
    skills: BTreeMap<String, Vec<(u64, String)>>,
}

impl<'a> ReadPass<'a> {
    fn new(window: &'a RunWindow<'a>, deadline: Instant, limits: ReadLimits) -> Self {
        Self {
            ids: &window.ids,
            recorded: &window.recorded,
            from: window.from,
            deadline,
            limits,
            files: 0,
            bytes: 0,
            entries: BTreeMap::new(),
            sessions: BTreeMap::new(),
            pauses: BTreeMap::new(),
            attention: BTreeMap::new(),
            skills: BTreeMap::new(),
        }
    }
}

impl ReadPass<'_> {
    /// Whether a log was last written well before the earliest run of interest, so it cannot hold its
    /// work. File times are coarse and clocks differ, so there is plenty of slack.
    fn stale(&self, path: &Path) -> bool {
        modified_ms(path).is_some_and(|m| m.saturating_add(MTIME_SLACK_MS) < self.from)
    }

    /// Whether to stop reading: out of time, or past the limit on the number or size of the logs.
    fn over_limit(&mut self, out: &mut ReadResult) -> bool {
        if Instant::now() > self.deadline {
            out.timed_out = true;
        }
        self.files += 1;
        if self.files > self.limits.files || self.bytes > self.limits.bytes {
            out.truncated = true;
        }
        out.timed_out || out.truncated
    }

    /// The text of a log, or a note about why not. Only a session that was recorded for a run is
    /// named in a note, because only then is it known to matter.
    fn load(&mut self, path: &Path, session: &str, out: &mut ReadResult) -> Option<String> {
        let recorded = self.recorded.contains(session);
        let mut note = |text: &str| {
            if recorded {
                out.notes.push(Note {
                    session: Some(session.to_string()),
                    text: text.to_string(),
                });
            }
        };
        match read_text(path) {
            Ok(LogFile::Content(text)) => {
                self.bytes += text.len() as u64;
                Some(text)
            }
            Ok(LogFile::Skipped) => None,
            Ok(LogFile::TooBig) => {
                note("a session log larger than 64 MiB was not read");
                None
            }
            Err(error) => {
                note(&format!("a session log could not be read: {error}"));
                None
            }
        }
    }

    fn add(
        &mut self,
        id: &str,
        mentions: HashSet<String>,
        parsed: LogUsage,
        agent: &str,
        out: &mut ReadResult,
    ) {
        // An id comes from the log's file or folder name or its own metadata: only a plain one is kept.
        let plain = (1..=MAX_NAME_LEN).contains(&id.len())
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
        if !plain {
            return;
        }
        let mut note = |text: String| {
            out.notes.push(Note {
                session: Some(id.to_string()),
                text,
            })
        };
        if parsed.malformed > 0 {
            note(format!(
                "{} malformed {agent} log line(s) were skipped",
                parsed.malformed
            ));
        }
        if parsed.unusable > 0 {
            note(format!(
                "{} {agent} usage line(s) had no usable model, time or count and were skipped",
                parsed.unusable
            ));
        }
        if parsed.unidentified > 0 {
            note(format!(
                "{} {agent} usage entr(ies) have no response id, so a repeat of them cannot be detected",
                parsed.unidentified
            ));
        }
        if parsed.entries.is_empty() {
            note(format!(
                "a {agent} session log held no usage entries in a recognised format"
            ));
        }
        for (key, entry) in parsed.entries {
            match self.entries.get_mut(&key) {
                Some(existing) => merge_largest(&mut existing.tokens, &entry.tokens),
                None => {
                    self.entries.insert(key, entry);
                }
            }
        }
        let (pauses, activity) = self.pauses.entry(id.to_string()).or_default();
        pauses.extend(parsed.pauses);
        activity.extend(parsed.activity);
        self.attention
            .entry(id.to_string())
            .or_default()
            .extend(parsed.attention);
        self.skills
            .entry(id.to_string())
            .or_default()
            .extend(parsed.skills);
        self.sessions
            .entry(id.to_string())
            .or_default()
            .extend(mentions);
    }
}

/// Reads the Codex and Copilot session logs that name a run in the window or were recorded for one.
fn read_other_sessions(
    roots: &Roots,
    window: &RunWindow,
    deadline: Instant,
    limits: ReadLimits,
) -> ReadResult {
    let mut out = ReadResult::default();
    let mut pass = ReadPass::new(window, deadline, limits);
    let sources = [
        (
            &roots.codex,
            read_codex as fn(&Path, &mut ReadPass, &mut ReadResult),
        ),
        (&roots.copilot, read_copilot),
    ];
    for (root, read) in sources {
        // The log folder itself may be a link (a dotfiles setup); what is inside it is not followed.
        if !root.is_dir() {
            continue;
        }
        out.roots_found += 1;
        read(root, &mut pass, &mut out);
        if out.timed_out || out.truncated {
            break;
        }
    }
    let mut by_session: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for entry in pass.entries.into_values() {
        by_session
            .entry(entry.session.clone())
            .or_default()
            .push(entry);
    }
    for (id, mentions) in pass.sessions {
        let mut entries = by_session.remove(&id).unwrap_or_default();
        entries.sort_by_key(|entry| entry.ts);
        let (mut pauses, mut activity) = pass.pauses.remove(&id).unwrap_or_default();
        pauses.sort_by_key(|(_, at)| *at);
        activity.sort_unstable();
        activity.dedup();
        let attention = pass.attention.remove(&id).unwrap_or_default();
        let mut skills = pass.skills.remove(&id).unwrap_or_default();
        skills.sort();
        skills.dedup();
        out.sessions.push(RunSession {
            id,
            mentions,
            entries,
            pauses,
            activity,
            attention,
            skills,
        });
    }
    out
}

/// A Claude Code session as usage sees it.
fn run_session(session: LogSession) -> RunSession {
    RunSession {
        id: session.id,
        mentions: session.claims,
        entries: session
            .responses
            .into_iter()
            .map(|response| Entry {
                session: response.session,
                ts: response.ts,
                model: response.model,
                tokens: response.tokens,
                subagent: response.subagent,
                effort: response.effort,
            })
            .collect(),
        pauses: session.pauses,
        activity: session.activity,
        attention: session.attention,
        skills: session.skills,
    }
}

/// Reads every session log that names a run in the window or was recorded for one, within `deadline`.
pub(crate) fn read_sessions(
    roots: &Roots,
    window: &RunWindow,
    deadline: Instant,
    limits: ReadLimits,
) -> ReadResult {
    let claude = read_claude(&roots.claude, window, deadline, limits);
    let mut out = if claude.timed_out || claude.truncated {
        ReadResult::default()
    } else {
        read_other_sessions(roots, window, deadline, limits)
    };
    out.roots_found += usize::from(claude.root_found);
    out.timed_out |= claude.timed_out;
    out.truncated |= claude.truncated;
    out.notes.extend(claude.notes);
    out.sessions
        .extend(claude.sessions.into_iter().map(run_session));
    out.sessions.sort_by(|a, b| a.id.cmp(&b.id));
    out
}
