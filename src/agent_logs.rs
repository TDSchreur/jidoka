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

mod claude;
mod codex;
pub(crate) mod copilot;

pub(crate) use claude::read_claude;
pub(crate) use codex::read_codex;
pub(crate) use copilot::read_copilot;

use crate::{
    install::agent_home,
    request_logs::{Variables, is_real_dir, result_bytes, run_claims, truncate},
    run::Run,
    usage::{PauseKind, Tokens, overlaps, parse_timestamp},
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

/// Something that limits what a session's usage covers.
#[derive(Debug)]
pub(crate) struct Note {
    /// The session it is about; `None` when it concerns the reading as a whole.
    pub(crate) session: Option<String>,
    pub(crate) text: String,
}

#[derive(Debug, Default)]
pub(crate) struct ReadResult {
    pub(crate) sessions: Vec<LogSession>,
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

/// The pause markers, moments of activity, attention signals and skills one session's logs gave: all but the
/// responses, which an adapter reads on their own.
#[derive(Default)]
struct Markers {
    pauses: Vec<(PauseKind, u64)>,
    activity: Vec<u64>,
    attention: Vec<(u64, crate::attention::Signal)>,
    skills: Vec<(u64, String)>,
}

impl Markers {
    fn absorb(&mut self, other: Markers) {
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

// --- what the adapters give ----------------------------------------------------------------------

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
    /// Whether the log says which tool calls it made. A Copilot usage record whose message is missing does not.
    pub(crate) calls_known: bool,
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

impl LogSession {
    /// Whether the session belongs to `run`: it executed a Jidoka command that names the run, or the run
    /// recorded the session (the Session claim in the glossary).
    pub(crate) fn belongs_to(&self, run: &Run) -> bool {
        self.claims.contains(&run.id) || run.sessions.contains(&self.id)
    }
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

/// What an adapter skipped in one session's logs. Lines that looked like a response but could not be used
/// are counted, so the notes can say so instead of silently looking complete.
#[derive(Default)]
struct Skipped {
    /// Lines that were not valid JSON.
    malformed: usize,
    /// Valid lines that held no usable model, time or counts.
    unusable: usize,
    /// Responses that carry no id, so a repeat of them cannot be told from a new call.
    unidentified: usize,
}

impl Skipped {
    fn add(&mut self, other: &Skipped) {
        self.malformed += other.malformed;
        self.unusable += other.unusable;
        self.unidentified += other.unidentified;
    }
}

/// The notes about what was skipped in `session`'s logs, and about a session that held no response at all.
fn skipped_notes(
    agent: &str,
    session: &str,
    skipped: &Skipped,
    empty: bool,
    notes: &mut Vec<Note>,
) {
    let mut note = |text: String| {
        notes.push(Note {
            session: Some(session.to_string()),
            text,
        });
    };
    if skipped.malformed > 0 {
        note(format!(
            "{} malformed {agent} log line(s) were skipped",
            skipped.malformed
        ));
    }
    if skipped.unusable > 0 {
        note(format!(
            "{} {agent} usage line(s) had no usable model, time or count and were skipped",
            skipped.unusable
        ));
    }
    if skipped.unidentified > 0 {
        note(format!(
            "{} {agent} usage entr(ies) have no response id, so a repeat of them cannot be detected",
            skipped.unidentified
        ));
    }
    if empty {
        note(format!(
            "a {agent} session log held no usage entries in a recognised format"
        ));
    }
}

/// Whether a log's own file or folder name is safe to keep as a session id: a plain one.
fn plain_id(id: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// `YYYY-MM-DD` for milliseconds since the epoch, to compare with the day folders.
pub(crate) fn civil_day(millis: u64) -> String {
    crate::usage::format_timestamp(millis)[..10].to_string()
}

// --- reading ----------------------------------------------------------------------------------------

/// One pass over one agent's logs: what to look for and the limits, and how much has been read so far.
struct ReadPass<'a> {
    recorded: &'a HashSet<&'a str>,
    from: u64,
    deadline: Instant,
    limits: ReadLimits,
    files: usize,
    bytes: u64,
}

impl<'a> ReadPass<'a> {
    fn new(window: &'a RunWindow<'a>, deadline: Instant, limits: ReadLimits) -> Self {
        Self {
            recorded: &window.recorded,
            from: window.from,
            deadline,
            limits,
            files: 0,
            bytes: 0,
        }
    }

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
}

/// Reads every session log that names a run in the window or was recorded for one, within `deadline`.
pub(crate) fn read_sessions(
    roots: &Roots,
    window: &RunWindow,
    deadline: Instant,
    limits: ReadLimits,
) -> ReadResult {
    let mut out = ReadResult::default();
    for read in [
        read_claude(&roots.claude, window, deadline, limits),
        read_codex(&roots.codex, window, deadline, limits),
        read_copilot(&roots.copilot, window, deadline, limits),
    ] {
        out.roots_found += usize::from(read.root_found);
        out.timed_out |= read.timed_out;
        out.truncated |= read.truncated;
        out.notes.extend(read.notes);
        out.sessions.extend(read.sessions);
    }
    out.sessions.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests;
