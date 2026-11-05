//! Readers for the session logs of Claude Code, Codex and Copilot. Each keeps only the model, the token
//! counts and the timestamp of a response, plus which run ids and session ids a log names. No conversation
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
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// The largest log read: real session logs are a few megabytes, so a bigger one is skipped.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Where each agent keeps its session logs.
#[derive(Clone, Debug)]
pub(crate) struct Roots {
    pub(crate) claude: PathBuf,
    pub(crate) codex: PathBuf,
    pub(crate) copilot: PathBuf,
}

impl Roots {
    /// The agents' usual folders under `home`, honoring `CLAUDE_CONFIG_DIR` and `CODEX_HOME` as install does.
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
}

// --- shared pieces ----------------------------------------------------------------------------------

/// A model name that is safe to store and print: a log is written by another program.
fn model_name(value: &Value) -> Option<String> {
    let name = value.as_str()?;
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphanumeric()
        && name.len() <= 100
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:/@+-".contains(c)))
    .then(|| name.to_string())
}

/// A token count, or `None` when the log holds no usable one.
fn count(value: Option<&Value>) -> Option<u64> {
    value?.as_u64().filter(|n| *n < (1 << 53))
}

fn timestamp(value: Option<&Value>) -> Option<u64> {
    parse_timestamp(value?.as_str()?)
}

fn modified_ms(path: &Path) -> Option<u64> {
    let modified = fs::symlink_metadata(path).ok()?.modified().ok()?;
    u64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

enum Read {
    Content(String),
    /// Not a regular file, so not followed.
    Skipped,
    TooBig,
}

/// The text of a regular log file; a link or special file is not followed.
fn read_text(path: &Path) -> io::Result<Read> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Ok(Read::Skipped);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Ok(Read::TooBig);
    }
    Ok(Read::Content(
        String::from_utf8_lossy(&fs::read(path)?).into_owned(),
    ))
}

/// The run ids a log names: whole numbers of 8 to 22 digits that are a known run id. A run id is
/// digits only, so a number inside a longer number or a word is not a match.
pub(crate) fn mentioned_runs(text: &str, ids: &HashSet<&str>) -> HashSet<String> {
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
        if (8..=22).contains(&(at - start)) {
            let number = &text[start..at];
            if ids.contains(number) {
                found.insert(number.to_string());
            }
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

/// A response is often written more than once, sometimes with growing counts: the largest of each field wins.
fn merge_largest(into: &mut Tokens, other: &Tokens) {
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

/// What one log file gave: keyed entries and how many lines that looked like usage could not be read.
#[derive(Default)]
struct Parsed {
    entries: Vec<(String, Entry)>,
    malformed: usize,
}

fn is_unrecognised(parsed: &Parsed) -> bool {
    parsed.entries.is_empty()
}

// --- Claude Code ------------------------------------------------------------------------------------

/// Claude Code writes a response once per content block with the same usage, so a response counts once.
/// `input_tokens` excludes the cache; thinking tokens are inside `output_tokens`.
fn parse_claude(text: &str, session: &str, subagent_file: bool) -> Parsed {
    let mut parsed = Parsed::default();
    for line in text.lines().filter(|line| line.contains("\"usage\"")) {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            parsed.malformed += 1;
            continue;
        };
        let message = &data["message"];
        let usage = &message["usage"];
        let (Some(model), Some(ts)) = (
            model_name(&message["model"]),
            timestamp(data.get("timestamp")),
        ) else {
            continue;
        };
        if !usage.is_object() {
            continue;
        }
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
        let id = message["id"]
            .as_str()
            .or(data["uuid"].as_str())
            .map_or_else(|| ts.to_string(), str::to_string);
        let key = format!("claude|{id}|{}", data["requestId"].as_str().unwrap_or(""));
        let subagent = subagent_file || data["isSidechain"].as_bool() == Some(true);
        parsed.entries.push((
            key,
            Entry {
                session: session.to_string(),
                ts,
                model,
                tokens,
                subagent,
            },
        ));
    }
    parsed
}

fn read_claude(root: &Path, ctx: &mut Ctx, out: &mut ReadResult) {
    for slug in sorted_dir(root).into_iter().filter(|e| e.path().is_dir()) {
        for file in sorted_dir(&slug.path()) {
            let path = file.path();
            let Some(stem) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".jsonl"))
            else {
                continue;
            };
            if !path.is_file() {
                continue;
            }
            if ctx.over_time(out) {
                return;
            }
            if modified_ms(&path).is_some_and(|m| m < ctx.from) {
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
            // The subagents of a session are in its own folder; the same response is never counted twice.
            let sub_dir = slug.path().join(&stem).join("subagents");
            for sub in sorted_dir(&sub_dir) {
                let sub_path = sub.path();
                if sub_path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                if let Some(sub_text) = ctx.load(&sub_path, &stem, out) {
                    let more = parse_claude(&sub_text, &stem, true);
                    parsed.malformed += more.malformed;
                    parsed.entries.extend(more.entries);
                }
            }
            ctx.add(&stem, mentions, parsed, "Claude Code", out);
        }
    }
}

// --- Codex ------------------------------------------------------------------------------------------

struct Rollout {
    id: String,
    root: String,
    subagent: bool,
    mentions: HashSet<String>,
    text: String,
}

/// The ids on a rollout's `session_meta` line. A rollout of a subagent has its own id and the id of the
/// root session it belongs to.
fn codex_meta(text: &str) -> Option<(String, String, bool)> {
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

/// Codex counts cached input inside `input_tokens`; the cached part is moved to cache read, and cache
/// writes are taken out of input too. Reasoning tokens are part of output.
fn parse_codex(text: &str, session: &str, subagent: bool) -> Parsed {
    let mut parsed = Parsed::default();
    let mut model_by_turn: BTreeMap<String, String> = BTreeMap::new();
    let mut last_model: Option<String> = None;
    for line in text.lines() {
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
                if let Some(turn) = payload["turn_id"].as_str() {
                    model_by_turn.insert(turn.to_string(), model.clone());
                }
                last_model = Some(model);
            }
            continue;
        }
        if data["type"] != "token_usage_record" {
            continue;
        }
        let usage = &payload["usage"];
        let (Some(ts), true) = (timestamp(data.get("timestamp")), usage.is_object()) else {
            continue;
        };
        let (input, cached, written, output) = (
            count(usage.get("input_tokens")),
            count(usage.get("cached_input_tokens")),
            count(usage.get("cache_write_input_tokens")),
            count(usage.get("output_tokens")),
        );
        if input.is_none() && output.is_none() {
            continue;
        }
        let model = payload["turn_id"]
            .as_str()
            .and_then(|t| model_by_turn.get(t))
            .or(last_model.as_ref());
        let Some(model) = model else { continue };
        let tokens = Tokens {
            input: input.map(|n| {
                n.saturating_sub(cached.unwrap_or(0))
                    .saturating_sub(written.unwrap_or(0))
            }),
            cache_read: cached,
            cache_write: written,
            output,
            reasoning: count(usage.get("reasoning_output_tokens")),
        };
        let id = payload["response_id"]
            .as_str()
            .map_or_else(|| format!("{ts}|{input:?}|{output:?}"), str::to_string);
        parsed.entries.push((
            format!("codex|{id}"),
            Entry {
                session: session.to_string(),
                ts,
                model: model.clone(),
                tokens,
                subagent,
            },
        ));
    }
    parsed
}

fn read_codex(root: &Path, ctx: &mut Ctx, out: &mut ReadResult) {
    let earliest_day = civil_day(ctx.from.saturating_sub(DAY_MS));
    let mut rollouts: Vec<Rollout> = Vec::new();
    for year in sorted_dir(root).into_iter().filter(|e| e.path().is_dir()) {
        for month in sorted_dir(&year.path())
            .into_iter()
            .filter(|e| e.path().is_dir())
        {
            for day in sorted_dir(&month.path())
                .into_iter()
                .filter(|e| e.path().is_dir())
            {
                let name = format!(
                    "{}-{}-{}",
                    year.file_name().to_string_lossy(),
                    month.file_name().to_string_lossy(),
                    day.file_name().to_string_lossy()
                );
                if name < earliest_day {
                    continue;
                }
                for file in sorted_dir(&day.path()) {
                    let path = file.path();
                    let fname = file.file_name().to_string_lossy().into_owned();
                    if !(fname.starts_with("rollout-") && fname.ends_with(".jsonl"))
                        || !path.is_file()
                    {
                        continue;
                    }
                    if ctx.over_time(out) {
                        return;
                    }
                    if modified_ms(&path).is_some_and(|m| m < ctx.from) {
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
                        id,
                        root,
                        subagent,
                        mentions,
                        text,
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
        let parsed = parse_codex(&rollout.text, &rollout.root, rollout.subagent);
        let (mentions, all) = by_root.entry(rollout.root.clone()).or_default();
        mentions.extend(rollout.mentions.iter().cloned());
        all.malformed += parsed.malformed;
        all.entries.extend(parsed.entries);
    }
    for (root, (mentions, parsed)) in by_root {
        ctx.add(&root, mentions, parsed, "Codex", out);
    }
}

/// `YYYY-MM-DD` for milliseconds since the epoch, to compare with the day folders.
fn civil_day(millis: u64) -> String {
    crate::usage::format_timestamp(millis)[..10].to_string()
}

// --- Copilot ----------------------------------------------------------------------------------------

/// Copilot counts cache reads and writes inside `inputTokens`; both are taken out of input. Reasoning
/// tokens are part of output.
fn parse_copilot(text: &str, session: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for line in text
        .lines()
        .filter(|line| line.contains("\"session.usage_record\""))
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
            continue;
        };
        let (input, read, write, output) = (
            count(usage.get("inputTokens")),
            count(usage.get("cacheReadTokens")),
            count(usage.get("cacheWriteTokens")),
            count(usage.get("outputTokens")),
        );
        if input.is_none() && output.is_none() {
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
        let call = usage["apiCallId"]
            .as_str()
            .map_or_else(|| format!("{ts}|{input:?}|{output:?}"), str::to_string);
        parsed.entries.push((
            format!("copilot|{session}|{call}"),
            Entry {
                session: session.to_string(),
                ts,
                model,
                tokens,
                subagent: usage["initiator"] == "sub-agent",
            },
        ));
    }
    parsed
}

fn read_copilot(root: &Path, ctx: &mut Ctx, out: &mut ReadResult) {
    for dir in sorted_dir(root).into_iter().filter(|e| e.path().is_dir()) {
        let session = dir.file_name().to_string_lossy().into_owned();
        let path = dir.path().join("events.jsonl");
        if !path.is_file() {
            continue;
        }
        if ctx.over_time(out) {
            return;
        }
        if modified_ms(&path).is_some_and(|m| m < ctx.from) {
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

/// What the readers share: which runs and sessions are of interest, and the time budget.
struct Ctx<'a> {
    ids: &'a HashSet<&'a str>,
    recorded: &'a HashSet<&'a str>,
    from: u64,
    deadline: Instant,
    /// Entries by key across every session, so a response that appears twice counts once.
    entries: BTreeMap<String, Entry>,
    /// Per session: the run ids it names and the keys of its entries.
    sessions: BTreeMap<String, HashSet<String>>,
}

impl Ctx<'_> {
    fn over_time(&self, out: &mut ReadResult) -> bool {
        if Instant::now() > self.deadline {
            out.timed_out = true;
        }
        out.timed_out
    }

    /// The text of a log, or a note about why not. Only a session that was recorded for a run is
    /// named in a note, because only then is it known to matter.
    fn load(&self, path: &Path, session: &str, out: &mut ReadResult) -> Option<String> {
        let note_for = |text: &str, out: &mut ReadResult| {
            if self.recorded.contains(session) {
                out.notes.push(Note {
                    session: Some(session.to_string()),
                    text: text.to_string(),
                });
            }
        };
        match read_text(path) {
            Ok(Read::Content(text)) => Some(text),
            Ok(Read::Skipped) => None,
            Ok(Read::TooBig) => {
                note_for("a session log larger than 64 MiB was not read", out);
                None
            }
            Err(_) => {
                note_for("a session log could not be read", out);
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
        if parsed.malformed > 0 {
            out.notes.push(Note {
                session: Some(id.to_string()),
                text: format!(
                    "{} malformed {agent} log line(s) were skipped",
                    parsed.malformed
                ),
            });
        }
        if is_unrecognised(&parsed) {
            out.notes.push(Note {
                session: Some(id.to_string()),
                text: format!("a {agent} session log held no usage entries in a recognised format"),
            });
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
) -> ReadResult {
    let mut out = ReadResult::default();
    let mut ctx = Ctx {
        ids,
        recorded,
        from,
        deadline,
        entries: BTreeMap::new(),
        sessions: BTreeMap::new(),
    };
    let _ = SystemTime::now();
    for (root, read) in [
        (
            &roots.claude,
            read_claude as fn(&Path, &mut Ctx, &mut ReadResult),
        ),
        (&roots.codex, read_codex),
        (&roots.copilot, read_copilot),
    ] {
        if !root.is_dir() {
            continue;
        }
        out.roots_found += 1;
        read(root, &mut ctx, &mut out);
        if out.timed_out {
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
