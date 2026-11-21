//! Reading a shell command line without running it: where its commands start, their words, and which runs
//! a Jidoka command in it names. Used to find the sessions that belong to a run and to classify requests.

use std::collections::{HashMap, HashSet};

/// The longest command read: a patch or a here-document can carry whole files.
pub(crate) const MAX_COMMAND_BYTES: usize = 64 * 1024;

/// `command` cut to `MAX_COMMAND_BYTES`, at a character boundary.
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

/// The run ids in `ids` that a Jidoka command in `command` names: `run update`, `run sync`, `run inspect`,
/// `run check`, `run stats`, `pr open`, `pr sync` or `pr wait` followed by the id. Only a command that runs `jidoka` counts: `grep "jidoka run update 1 "
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
            (
                Some(&"run"),
                Some(&("update" | "sync" | "inspect" | "check" | "stats"))
            ) | (Some(&"pr"), Some(&("open" | "sync" | "wait")))
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

/// One command of a shell line.
pub(crate) struct Segment {
    pub(crate) text: String,
    /// It only supports another command: it stands inside a command substitution, or after a `|` where it
    /// filters the output of the command before it. It still runs, but it is not the line's own work.
    pub(crate) plumbing: bool,
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
pub(crate) fn segments(command: &str) -> Vec<Segment> {
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
pub(crate) fn command_words(segment: &str) -> Vec<&str> {
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
