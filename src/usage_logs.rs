//! Readers for the session logs of Claude Code, Codex and Copilot. Each keeps only the model, the
//! reasoning effort (Claude Code and Codex record it), the token counts and the timestamp of a response, plus which run ids and session ids a log names. No conversation
//! text, prompt, tool output or credential leaves this module.
//!
//! Formats (see the README for what is validated): Claude Code transcripts and their subagent transcripts,
//! Codex rollout files, and Copilot `events.jsonl` files.

use crate::{
    install::agent_home,
    usage::{parse_timestamp, Entry, Tokens},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{Instant, UNIX_EPOCH},
};

/// The largest log read: real session logs are a few megabytes, so a bigger one is skipped.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const DAY_MS: u64 = 24 * 60 * 60 * 1000;
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
pub(crate) struct SessionData {
    pub(crate) id: String,
    /// The run ids the log names.
    pub(crate) mentions: HashSet<String>,
    pub(crate) entries: Vec<Entry>,
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
    pub(crate) sessions: Vec<SessionData>,
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
        && name.len() <= 100
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
/// `run inspect` or `run stats`, as `factory run update <id>` is written. Run ids also appear in tool
/// output and in files an agent reads (a session that listed every run would otherwise claim them all), so
/// a bare number does not count. A run id is digits only, so a number inside a longer number does not match.
pub(crate) fn mentioned_runs(text: &str, ids: &HashSet<&str>) -> HashSet<String> {
    const COMMANDS: [&str; 3] = ["run update", "run inspect", "run stats"];
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
        if COMMANDS.iter().any(|command| before.ends_with(command)) {
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
struct Parsed {
    entries: Vec<(String, Entry)>,
    /// Lines that were not valid JSON.
    malformed: usize,
    /// Valid lines that held no usable model, time or counts.
    unusable: usize,
    /// Entries that carry no response id, so a repeat of them cannot be told from a new call.
    unidentified: usize,
}

impl Parsed {
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

    fn absorb(&mut self, other: Parsed) {
        self.malformed += other.malformed;
        self.unusable += other.unusable;
        self.unidentified += other.unidentified;
        self.entries.extend(other.entries);
    }
}

// --- Claude Code ------------------------------------------------------------------------------------

/// Claude Code writes a response once per content block with the same usage, so a response counts once.
/// `input_tokens` excludes the cache; thinking tokens are inside `output_tokens`.
fn parse_claude(text: &str, session: &str, subagent_file: bool) -> Parsed {
    let mut parsed = Parsed::default();
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("\"usage\""))
    {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            parsed.malformed += 1;
            continue;
        };
        let message = &data["message"];
        let usage = &message["usage"];
        if !usage.is_object() {
            // Another kind of line that mentions "usage", such as a task's totals: not a response.
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
        let id = message["id"]
            .as_str()
            .or(data["uuid"].as_str())
            .map(|id| format!("{id}|{}", data["requestId"].as_str().unwrap_or("")));
        let subagent = subagent_file || data["isSidechain"].as_bool() == Some(true);
        parsed.push(
            id,
            number,
            "claude",
            Entry {
                session: session.to_string(),
                ts,
                model,
                tokens,
                subagent,
                effort: effort_name(&data["effort"]),
            },
        );
    }
    parsed
}

fn read_claude(root: &Path, ctx: &mut Ctx, out: &mut ReadResult) {
    for slug in sub_dirs(root) {
        for file in files_in(&slug.path()) {
            let path = file.path();
            let Some(stem) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".jsonl"))
            else {
                continue;
            };
            if ctx.over_limit(out) {
                return;
            }
            if ctx.stale(&path) {
                continue;
            }
            let stem = stem.to_string();
            let Some(text) = ctx.load(&path, &stem, out) else {
                continue;
            };
            let mentions = mentioned_runs(&text, ctx.ids);
            if mentions.is_empty() && !ctx.recorded.contains(stem.as_str()) {
                continue;
            }
            let mut parsed = parse_claude(&text, &stem, false);
            drop(text);
            // The subagents of a session are in its own folder; the same response is never counted twice.
            for sub in files_in(&slug.path().join(&stem).join("subagents")) {
                let sub_path = sub.path();
                if sub_path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                if ctx.over_limit(out) {
                    break;
                }
                if let Some(sub_text) = ctx.load(&sub_path, &stem, out) {
                    parsed.absorb(parse_claude(&sub_text, &stem, true));
                }
            }
            ctx.add(&stem, mentions, parsed, "Claude Code", out);
        }
    }
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
fn parse_codex(text: &str, session: &str, subagent: bool) -> Parsed {
    let mut parsed = Parsed::default();
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

fn read_codex(root: &Path, ctx: &mut Ctx, out: &mut ReadResult) {
    let earliest_day = civil_day(ctx.from.saturating_sub(DAY_MS));
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
                    if ctx.over_limit(out) {
                        return;
                    }
                    if ctx.stale(&path) {
                        continue;
                    }
                    let Some(text) = ctx.load(&path, &fname, out) else {
                        continue;
                    };
                    let Some((id, root, subagent)) = codex_meta(&text) else {
                        continue;
                    };
                    let mentions = mentioned_runs(&text, ctx.ids);
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
                || ctx.recorded.contains(r.id.as_str())
                || ctx.recorded.contains(r.root.as_str())
        })
        .map(|r| r.root.clone())
        .collect();
    let mut by_root: BTreeMap<String, (HashSet<String>, Parsed)> = BTreeMap::new();
    for rollout in rollouts.iter().filter(|r| claimed_roots.contains(&r.root)) {
        if ctx.over_limit(out) {
            return;
        }
        // Only a claimed rollout is read again, to parse it.
        let Some(text) = ctx.load(&rollout.path, &rollout.root, out) else {
            continue;
        };
        let parsed = parse_codex(&text, &rollout.root, rollout.subagent);
        let (mentions, all) = by_root.entry(rollout.root.clone()).or_default();
        mentions.extend(rollout.mentions.iter().cloned());
        all.absorb(parsed);
    }
    for (root, (mentions, parsed)) in by_root {
        ctx.add(&root, mentions, parsed, "Codex", out);
    }
}

/// `YYYY-MM-DD` for milliseconds since the epoch, to compare with the day folders.
pub(crate) fn civil_day(millis: u64) -> String {
    crate::usage::format_timestamp(millis)[..10].to_string()
}

// --- Copilot ----------------------------------------------------------------------------------------

/// Copilot counts cache reads and writes inside `inputTokens`; both are taken out of input. Reasoning
/// tokens are part of output.
fn parse_copilot(text: &str, session: &str) -> Parsed {
    let mut parsed = Parsed::default();
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
        let (input, read, write, output) = (
            count(usage.get("inputTokens")),
            count(usage.get("cacheReadTokens")),
            count(usage.get("cacheWriteTokens")),
            count(usage.get("outputTokens")),
        );
        if input.is_none() && output.is_none() {
            parsed.unusable += 1;
            continue;
        }
        let tokens = Tokens {
            input: input.map(|n| {
                n.saturating_sub(read.unwrap_or(0))
                    .saturating_sub(write.unwrap_or(0))
            }),
            cache_read: read,
            cache_write: write,
            output,
            reasoning: count(usage.get("reasoningTokens")),
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
                // Copilot's usage records carry no effort.
                effort: None,
            },
        );
    }
    parsed
}

fn read_copilot(root: &Path, ctx: &mut Ctx, out: &mut ReadResult) {
    for dir in sub_dirs(root) {
        let session = dir.file_name().to_string_lossy().into_owned();
        let path = dir.path().join("events.jsonl");
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        if ctx.over_limit(out) {
            return;
        }
        if ctx.stale(&path) {
            continue;
        }
        let Some(text) = ctx.load(&path, &session, out) else {
            continue;
        };
        let mentions = mentioned_runs(&text, ctx.ids);
        if mentions.is_empty() && !ctx.recorded.contains(session.as_str()) {
            continue;
        }
        let parsed = parse_copilot(&text, &session);
        ctx.add(&session, mentions, parsed, "Copilot", out);
    }
}

// --- reading ----------------------------------------------------------------------------------------

/// What the readers share: which runs and sessions are of interest, and the limits.
struct Ctx<'a> {
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
}

impl Ctx<'_> {
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
            Err(_) => {
                note("a session log could not be read");
                None
            }
        }
    }

    fn add(
        &mut self,
        id: &str,
        mentions: HashSet<String>,
        parsed: Parsed,
        agent: &str,
        out: &mut ReadResult,
    ) {
        // An id comes from the log's file or folder name or its own metadata: only a plain one is kept.
        let plain = (1..=100).contains(&id.len())
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
        self.sessions
            .entry(id.to_string())
            .or_default()
            .extend(mentions);
    }
}

/// Reads every session log that names a run in `ids` or was recorded in `recorded`, within `deadline`.
pub(crate) fn read_sessions(
    roots: &Roots,
    ids: &HashSet<&str>,
    recorded: &HashSet<&str>,
    from: u64,
    deadline: Instant,
    limits: ReadLimits,
) -> ReadResult {
    let mut out = ReadResult::default();
    let mut ctx = Ctx {
        ids,
        recorded,
        from,
        deadline,
        limits,
        files: 0,
        bytes: 0,
        entries: BTreeMap::new(),
        sessions: BTreeMap::new(),
    };
    for (root, read) in [
        (
            &roots.claude,
            read_claude as fn(&Path, &mut Ctx, &mut ReadResult),
        ),
        (&roots.codex, read_codex),
        (&roots.copilot, read_copilot),
    ] {
        // The log folder itself may be a link (a dotfiles setup); what is inside it is not followed.
        if !root.is_dir() {
            continue;
        }
        out.roots_found += 1;
        read(root, &mut ctx, &mut out);
        if out.timed_out || out.truncated {
            break;
        }
    }
    let mut by_session: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for entry in ctx.entries.into_values() {
        by_session
            .entry(entry.session.clone())
            .or_default()
            .push(entry);
    }
    for (id, mentions) in ctx.sessions {
        let mut entries = by_session.remove(&id).unwrap_or_default();
        entries.sort_by_key(|entry| entry.ts);
        out.sessions.push(SessionData {
            id,
            mentions,
            entries,
        });
    }
    out
}
