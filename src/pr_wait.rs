//! `jidoka pr wait`: the options and the outcome of watching the checks of a run's pull request. The pull
//! request host watches them (`PrHost::watch`).

use crate::{inspect::shown, pr_state::PrState};
use serde::Serialize;
use std::time::{Duration, Instant};

/// Time, so that tests never sleep.
pub(crate) trait Clock {
    fn elapsed(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    Passed,
    Failed,
    Cancelled,
    Timeout,
    HeadChanged,
    NoCi,
    NotOpen,
}

impl Outcome {
    /// The exit status of the command. 1 is left for problems with the tools or the run.
    pub(crate) fn exit_code(self) -> u8 {
        match self {
            Outcome::Passed => 0,
            Outcome::Failed => 2,
            Outcome::Cancelled => 3,
            Outcome::Timeout => 4,
            Outcome::HeadChanged => 5,
            Outcome::NoCi => 6,
            Outcome::NotOpen => 7,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BaseState {
    UpToDate,
    Behind,
    Conflicts,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CheckResult {
    pub(crate) name: String,
    pub(crate) bucket: String,
    pub(crate) link: String,
}

/// A failed check, with the end of its log when it could be fetched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Failure {
    pub(crate) name: String,
    pub(crate) link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) excerpt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Waited {
    pub(crate) outcome: Outcome,
    pub(crate) exit_code: u8,
    pub(crate) pull_request: String,
    pub(crate) head: String,
    /// The head commit the pull request has now, when it changed while waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) new_head: Option<String>,
    pub(crate) base: BaseState,
    pub(crate) checks: Vec<CheckResult>,
    /// The failed checks, when the outcome is `failed`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) failures: Vec<Failure>,
    /// The state the pull request had when the wait ended. Not part of the JSON output.
    #[serde(skip)]
    pub(crate) state: PrState,
}

/// How long to wait for the checks when the caller says nothing.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The longest `--timeout` that is accepted.
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub(crate) timeout: Duration,
    /// Return `failed` at the first failing check instead of waiting for the others.
    pub(crate) fail_fast: bool,
    /// The commit the run's branch has on origin, when its worktree is on that commit. Right after a push,
    /// the host can report the previous head for a few seconds; the wait gives it a short grace to catch up.
    pub(crate) pushed_head: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            fail_fast: false,
            pushed_head: None,
        }
    }
}

/// Seconds in a minute, the `m` unit of `--timeout`.
const SECONDS_PER_MINUTE: u64 = 60;
/// Seconds in an hour, the `h` unit of `--timeout`.
const SECONDS_PER_HOUR: u64 = 3600;

/// A failing check's name is cut to this many characters so one long matrix name cannot flood the dashboard.
const MAX_CHECK_NAME_CHARS: usize = 50;

/// `90s`, `30m`, `2h` or a plain number of seconds, from one second to 24 hours.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let invalid =
        || format!("Invalid --timeout {text:?}: use a number with s, m or h, such as 90s or 30m");
    let (digits, unit) = match text.chars().last() {
        Some('s') => (&text[..text.len() - 1], 1),
        Some('m') => (&text[..text.len() - 1], SECONDS_PER_MINUTE),
        Some('h') => (&text[..text.len() - 1], SECONDS_PER_HOUR),
        _ => (text, 1),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let seconds = digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(unit))
        .ok_or_else(invalid)?;
    let duration = Duration::from_secs(seconds);
    if duration.is_zero() || duration > MAX_TIMEOUT {
        return Err(format!(
            "Invalid --timeout {text:?}: use between 1 second and 24 hours"
        ));
    }
    Ok(duration)
}

/// `[--timeout <duration>] [--fail-fast] [--json]` in any order: the options of `jidoka pr wait`, and
/// whether to print JSON.
pub(crate) fn parse_options(options: &[&str]) -> Result<(Options, bool), String> {
    let (mut parsed, mut json, mut timeout_given) = (Options::default(), false, false);
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--json" if !json => json = true,
            "--fail-fast" if !parsed.fail_fast => parsed.fail_fast = true,
            "--timeout" if !timeout_given => {
                timeout_given = true;
                parsed.timeout = parse_duration(args.next().copied().unwrap_or(""))?;
            }
            other => return Err(format!("Unknown or repeated option {other}")),
        }
    }
    Ok((parsed, json))
}

/// The real clock.
pub(crate) struct RealClock(pub(crate) Instant);

impl Clock for RealClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// The outcome of `waited` as the run's activity.
pub(crate) fn activity_text(waited: &Waited) -> String {
    match waited.outcome {
        Outcome::Passed => "CI passed".into(),
        Outcome::Failed => {
            // A few names are enough for the dashboard; a matrix can fail dozens of jobs.
            let names: Vec<String> = waited
                .checks
                .iter()
                .filter(|check| check.bucket == "fail")
                .map(|check| {
                    shown(&check.name)
                        .chars()
                        .take(MAX_CHECK_NAME_CHARS)
                        .collect()
                })
                .collect();
            let more = names.len().saturating_sub(3);
            let listed = names[..names.len() - more].join(", ");
            if more > 0 {
                format!("CI failed: {listed} and {more} more")
            } else {
                format!("CI failed: {listed}")
            }
        }
        Outcome::Cancelled => "CI cancelled".into(),
        Outcome::Timeout => "CI timed out".into(),
        Outcome::HeadChanged => "Head commit changed while waiting for CI".into(),
        Outcome::NoCi => "No CI checks".into(),
        Outcome::NotOpen => "Pull request is not open".into(),
    }
}

/// The commit `branch` has on origin, when the worktree's `HEAD` is that commit; `None` otherwise, or when
/// git cannot tell. It is only a hint for the wait, so a git problem is not reported.
pub(crate) fn pushed_head(worktree: &std::path::Path, branch: &str) -> Option<String> {
    let rev = |name: &str| {
        crate::run_begin::git(worktree, &["rev-parse", "--verify", "--quiet", name]).ok()
    };
    let head = rev("HEAD^{commit}")?;
    (rev(&format!("refs/remotes/origin/{branch}^{{commit}}"))? == head).then_some(head)
}

/// The outcome and what the agent needs to act on it, as text.
pub(crate) fn render(waited: &Waited) -> String {
    let mut out = format!("{}\n", activity_text(waited));
    out.push_str(&format!(
        "  Pull request: {}\n  Head commit: {}\n",
        shown(&waited.pull_request),
        shown(&waited.head.chars().take(7).collect::<String>())
    ));
    if let Some(new_head) = &waited.new_head {
        out.push_str(&format!(
            "  Head commit now: {}. Watch the new commit with `jidoka pr wait` again.\n",
            shown(&new_head.chars().take(7).collect::<String>())
        ));
    }
    for check in &waited.checks {
        out.push_str(&format!(
            "  {:<9} {}  {}\n",
            shown(&check.bucket),
            shown(&check.name),
            shown(&check.link)
        ));
    }
    for failure in &waited.failures {
        if let Some(text) = &failure.excerpt {
            out.push_str(&format!("  End of the log of {}:\n", shown(&failure.name)));
            for line in text.lines() {
                out.push_str(&format!("    | {}\n", shown(line)));
            }
        }
    }
    // The base state says nothing when the pull request is not open or its head moved.
    if matches!(waited.outcome, Outcome::NotOpen | Outcome::HeadChanged) {
        return out;
    }
    out.push_str(match waited.base {
        BaseState::UpToDate => "  The branch is up to date with its base.\n",
        BaseState::Behind => {
            "  The branch is behind its base: merge the base into it before calling the pull request ready.\n"
        }
        BaseState::Conflicts => {
            "  The branch conflicts with its base: resolve it before calling the pull request ready.\n"
        }
        BaseState::Unknown => "  The host has not said whether the branch is behind its base; check it before calling the pull request ready.\n",
    });
    out
}
