//! `factory pr wait`: watches the checks of a run's pull request and returns one CI outcome.

use crate::inspect::shown;
use serde::Serialize;
use std::{
    io,
    process::Command,
    time::{Duration, Instant},
};

/// What `gh` printed.
#[derive(Clone, Debug)]
pub(crate) struct GhOutput {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[derive(Debug)]
pub(crate) enum GhError {
    NotInstalled,
    Other(String),
}

#[path = "pr_wait_azure.rs"]
mod azure;

/// Runs host tools. The real implementation starts programs; tests answer from a script.
pub(crate) trait Gh {
    fn run(&self, args: &[&str]) -> Result<GhOutput, GhError>;
    fn az(&self, _args: &[&str]) -> Result<GhOutput, GhError> {
        Err(GhError::NotInstalled)
    }
}

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
}

/// A problem with the tools or the run, not a CI outcome. The message says what to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ToolProblem(pub(crate) String);

/// How long to wait for the checks when the caller says nothing.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The longest `--timeout` that is accepted.
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
/// How long checks may take to appear before the pull request counts as having no CI.
const GRACE: Duration = Duration::from_secs(90);
/// How long the pull request may take to show a commit that was just pushed, and how often it is asked.
const HEAD_GRACE: Duration = Duration::from_secs(30);
const HEAD_POLL: Duration = Duration::from_secs(5);
/// The most runs whose failed log is fetched.
const MAX_LOGS: usize = 3;
/// The most lines and bytes kept of a failed log: its end is where the cause is.
const LOG_LINES: usize = 40;
const LOG_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub(crate) timeout: Duration,
    /// Return `failed` at the first failing check instead of waiting for the others.
    pub(crate) fail_fast: bool,
    /// The commit the run's branch has on origin, when its worktree is on that commit. Right after a push,
    /// GitHub can report the previous head for a few seconds; the wait gives it `HEAD_GRACE` to catch up.
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

/// `90s`, `30m`, `2h` or a plain number of seconds, from one second to 24 hours.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let invalid =
        || format!("Invalid --timeout {text:?}: use a number with s, m or h, such as 90s or 30m");
    let (digits, unit) = match text.chars().last() {
        Some('s') => (&text[..text.len() - 1], 1),
        Some('m') => (&text[..text.len() - 1], 60),
        Some('h') => (&text[..text.len() - 1], 3600),
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

/// `[--timeout <duration>] [--fail-fast] [--json]` in any order: the options of `factory pr wait`, and
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

/// The real `gh`.
pub(crate) struct RealGh;

impl Gh for RealGh {
    fn az(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        let output = Command::new("az")
            .args(args)
            .env("AZURE_CORE_ONLY_SHOW_ERRORS", "true")
            .env("AZURE_EXTENSION_USE_DYNAMIC_INSTALL", "no")
            .output()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    GhError::NotInstalled
                } else {
                    GhError::Other(error.to_string())
                }
            })?;
        Ok(GhOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
    fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        let output = Command::new("gh")
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .output()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    GhError::NotInstalled
                } else {
                    GhError::Other(error.to_string())
                }
            })?;
        Ok(GhOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
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

/// Checks that `gh` can be started, so that its absence is reported before anything else is.
pub(crate) fn check_gh(gh: &dyn Gh) -> Result<(), ToolProblem> {
    run_gh(gh, &["--version"]).map(|_| ())
}

/// How often the checks are asked for.
const POLL: Duration = Duration::from_secs(15);
/// How long to wait before asking again when GitHub has not worked out the base state yet.
const BASE_RETRY: Duration = Duration::from_secs(3);
/// How many more times an unknown base state is asked for.
const BASE_RETRIES: usize = 5;

/// What `gh pr view` says about the pull request.
struct View {
    head: String,
    state: String,
    merge: String,
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
}

/// A failed `gh` call as a message that says what to do.
pub(crate) fn problem(what: &str, output: &GhOutput) -> ToolProblem {
    let stderr = output.stderr.to_lowercase();
    if output.code == Some(4) || stderr.contains("gh auth login") {
        return ToolProblem(
            "The GitHub CLI is not signed in. Run `gh auth login` and try again.".into(),
        );
    }
    let detail = shown(first_line(if output.stderr.trim().is_empty() {
        &output.stdout
    } else {
        &output.stderr
    }));
    if stderr.contains("error connecting")
        || stderr.contains("could not resolve host")
        || stderr.contains("no such host")
        || stderr.contains("i/o timeout")
    {
        return ToolProblem(format!(
            "Could not reach GitHub. Check your network connection and try again. ({detail})"
        ));
    }
    ToolProblem(format!("`gh {what}` failed: {detail}"))
}

pub(crate) fn run_gh(gh: &dyn Gh, args: &[&str]) -> Result<GhOutput, ToolProblem> {
    gh.run(args).map_err(|error| match error {
        GhError::NotInstalled => ToolProblem(
            "The GitHub CLI (`gh`) is not installed or not on PATH. Install it from https://cli.github.com, run `gh auth login`, and try again."
                .into(),
        ),
        GhError::Other(text) => ToolProblem(format!("Could not run `gh`: {text}")),
    })
}

fn read_view(gh: &dyn Gh, pull_request: &str) -> Result<View, ToolProblem> {
    let args = [
        "pr",
        "view",
        pull_request,
        "--json",
        "headRefOid,state,mergeStateStatus",
    ];
    let output = run_gh(gh, &args)?;
    if output.code != Some(0) {
        return Err(problem("pr view", &output));
    }
    let data: serde_json::Value = serde_json::from_str(&output.stdout).map_err(|error| {
        ToolProblem(format!(
            "`gh pr view` gave output that is not JSON: {error}"
        ))
    })?;
    let text = |name: &str| data[name].as_str().unwrap_or("").to_string();
    Ok(View {
        head: text("headRefOid"),
        state: text("state"),
        merge: text("mergeStateStatus"),
    })
}

fn read_checks(gh: &dyn Gh, pull_request: &str) -> Result<Vec<CheckResult>, ToolProblem> {
    let args = [
        "pr",
        "checks",
        pull_request,
        "--json",
        "name,bucket,state,link",
    ];
    let output = run_gh(gh, &args)?;
    // `gh pr checks` exits non-zero while checks are pending or failed; the JSON is what counts.
    let stdout = output.stdout.trim();
    if !stdout.starts_with('[') {
        if output.stderr.to_lowercase().contains("no checks reported") {
            return Ok(Vec::new());
        }
        return Err(problem("pr checks", &output));
    }
    let items: Vec<serde_json::Value> = serde_json::from_str(stdout).map_err(|error| {
        ToolProblem(format!(
            "`gh pr checks` gave output that is not JSON: {error}"
        ))
    })?;
    Ok(items
        .iter()
        .map(|item| {
            let text = |name: &str| item[name].as_str().unwrap_or("").to_string();
            CheckResult {
                name: text("name"),
                bucket: text("bucket"),
                link: text("link"),
            }
        })
        .collect())
}

/// The base state from `mergeStateStatus`; an unknown one is asked for again.
fn base_state(
    gh: &dyn Gh,
    clock: &dyn Clock,
    pull_request: &str,
    mut merge: String,
) -> Result<BaseState, ToolProblem> {
    for attempt in 0..=BASE_RETRIES {
        match merge.as_str() {
            "BEHIND" => return Ok(BaseState::Behind),
            "DIRTY" => return Ok(BaseState::Conflicts),
            "UNKNOWN" | "" if attempt < BASE_RETRIES => {
                clock.sleep(BASE_RETRY);
                merge = read_view(gh, pull_request)?.merge;
            }
            "UNKNOWN" | "" => return Ok(BaseState::Unknown),
            _ => return Ok(BaseState::UpToDate),
        }
    }
    Ok(BaseState::Unknown)
}

fn activity_text(waited: &Waited) -> String {
    match waited.outcome {
        Outcome::Passed => "CI passed".into(),
        Outcome::Failed => {
            // A few names are enough for the dashboard; a matrix can fail dozens of jobs.
            let names: Vec<String> = waited
                .checks
                .iter()
                .filter(|check| check.bucket == "fail")
                .map(|check| shown(&check.name).chars().take(50).collect())
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

/// The run id and the repository of a check link such as `https://github.com/o/r/actions/runs/123/job/4`.
/// The repository is written as `gh --repo` takes it: `owner/repo`, or `host/owner/repo` away from github.com.
fn run_of_link(link: &str) -> Option<(&str, String)> {
    let rest = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))?;
    let (place, tail) = rest.split_once("/actions/runs/")?;
    let id = tail.split(['/', '?', '#']).next()?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let name_ok = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-:".contains(c))
    };
    // The host and the owner start the `--repo` value, so they must not look like an option; a repository
    // may start with a dot (`.github`), but `.` and `..` are paths.
    let starts_plain = |part: &str| {
        part.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
    };
    let parts: Vec<&str> = place.split('/').collect();
    let [host, owner, repo] = parts[..] else {
        return None;
    };
    if !(name_ok(host) && starts_plain(host) && name_ok(owner) && starts_plain(owner))
        || !name_ok(repo)
        || matches!(repo, "." | "..")
    {
        return None;
    }
    let repository = if host == "github.com" {
        format!("{owner}/{repo}")
    } else {
        format!("{host}/{owner}/{repo}")
    };
    Some((id, repository))
}

/// The end of a log: its last lines, cut to a size that fits a screen.
fn excerpt(log: &str) -> Option<String> {
    let lines: Vec<&str> = log.lines().collect();
    let tail = lines[lines.len().saturating_sub(LOG_LINES)..].join("\n");
    if tail.trim().is_empty() {
        return None;
    }
    if tail.len() <= LOG_BYTES {
        return Some(tail);
    }
    let mut start = tail.len() - LOG_BYTES;
    while !tail.is_char_boundary(start) {
        start += 1;
    }
    Some(tail[start..].to_string())
}

/// The failed checks, each run's log fetched once (the first failed check of a run carries it), for at
/// most `MAX_LOGS` runs. A log that cannot be fetched leaves the name and link only.
fn failures(gh: &dyn Gh, checks: &[CheckResult]) -> Vec<Failure> {
    let mut fetched: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for check in checks.iter().filter(|check| check.bucket == "fail") {
        let mut text = None;
        if let Some((id, repository)) = run_of_link(&check.link) {
            if !fetched.contains(&id) && fetched.len() < MAX_LOGS {
                fetched.push(id);
                text = gh
                    .run(&["run", "view", id, "--repo", &repository, "--log-failed"])
                    .ok()
                    .filter(|output| output.code == Some(0))
                    .and_then(|output| excerpt(&output.stdout));
            }
        }
        out.push(Failure {
            name: check.name.clone(),
            link: check.link.clone(),
            excerpt: text,
        });
    }
    out
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

/// The status and draft flag of an Azure Repos pull request, read with `az` the way `wait` reads it.
pub(crate) fn azure_pr_status(gh: &dyn Gh, url: &str) -> Result<(String, bool), ToolProblem> {
    azure::status(gh, url)
}

/// The last source commit of the Azure Repos pull request at `url`.
pub(crate) fn azure_pr_source_commit(gh: &dyn Gh, url: &str) -> Result<String, ToolProblem> {
    azure::source_commit(gh, url)
}

pub(crate) fn azure_pr_merge_commit(gh: &dyn Gh, url: &str) -> Result<String, ToolProblem> {
    azure::merge_commit(gh, url)
}

/// Whether `url` is an Azure DevOps Services pull request URL, in its canonical or legacy form.
pub(crate) fn is_azure_pr_url(url: &str) -> bool {
    url.starts_with("https://dev.azure.com/")
        || url
            .strip_prefix("https://")
            .and_then(|rest| rest.split('/').next())
            .is_some_and(|host| host.ends_with(".visualstudio.com"))
}

/// Watches the checks of `pull_request` for its current head commit until they have ended, and returns the
/// CI outcome. `activity` is told what the run is doing, and `progress` how far the checks are. It only
/// reads: it never merges, pushes or updates the branch.
pub(crate) fn wait(
    pull_request: Option<&str>,
    options: &Options,
    gh: &dyn Gh,
    clock: &dyn Clock,
    activity: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(&str),
) -> Result<Waited, ToolProblem> {
    let pull_request = pull_request.ok_or_else(|| {
        ToolProblem(
            "The run has no pull request recorded. Open it, record it with `factory run update <id> finish running --pr <url>`, and try again."
                .into(),
        )
    })?;
    if is_azure_pr_url(pull_request) {
        return azure::wait(pull_request, options, gh, clock, activity, progress);
    }
    let mut first = read_view(gh, pull_request)?;
    // The whole wait, including the grace for a just-pushed head, stays within `--timeout`.
    let started = clock.elapsed();
    if let Some(pushed) = &options.pushed_head {
        while first.state == "OPEN"
            && first.head != *pushed
            && clock.elapsed().saturating_sub(started) < HEAD_GRACE.min(options.timeout)
        {
            progress("Waiting for the pull request to show the pushed commit");
            clock.sleep(HEAD_POLL);
            first = read_view(gh, pull_request)?;
        }
    }
    let result =
        |outcome: Outcome, view: &View, checks: Vec<CheckResult>, base: BaseState| Waited {
            outcome,
            exit_code: outcome.exit_code(),
            pull_request: pull_request.to_string(),
            head: first.head.clone(),
            new_head: (view.head != first.head).then(|| view.head.clone()),
            base,
            checks,
            failures: Vec::new(),
        };
    if first.state != "OPEN" {
        let waited = result(Outcome::NotOpen, &first, Vec::new(), BaseState::Unknown);
        activity(&activity_text(&waited));
        return Ok(waited);
    }
    activity("Waiting for CI");
    let waited = loop {
        let checks = read_checks(gh, pull_request)?;
        let view = read_view(gh, pull_request)?;
        if view.state != "OPEN" {
            break result(Outcome::NotOpen, &view, Vec::new(), BaseState::Unknown);
        }
        if view.head != first.head {
            // The checks belong to another commit; they say nothing about this one.
            break result(Outcome::HeadChanged, &view, Vec::new(), BaseState::Unknown);
        }
        let waited_for = clock.elapsed().saturating_sub(started);
        if checks.is_empty() {
            if waited_for >= GRACE.min(options.timeout) {
                let base = base_state(gh, clock, pull_request, view.merge.clone())?;
                break result(Outcome::NoCi, &view, checks, base);
            }
            progress("No checks reported yet");
            clock.sleep(POLL);
            continue;
        }
        let pending = checks
            .iter()
            .filter(|check| check.bucket == "pending")
            .count();
        let any_failed = checks.iter().any(|check| check.bucket == "fail");
        if pending > 0 && !(options.fail_fast && any_failed) {
            if waited_for >= options.timeout {
                let base = base_state(gh, clock, pull_request, view.merge.clone())?;
                break result(Outcome::Timeout, &view, checks, base);
            }
            progress(&format!(
                "{pending} of {} checks still running",
                checks.len()
            ));
            clock.sleep(POLL);
            continue;
        }
        let outcome = if any_failed {
            Outcome::Failed
        } else if checks.iter().any(|check| check.bucket == "cancel") {
            Outcome::Cancelled
        } else {
            Outcome::Passed
        };
        let base = base_state(gh, clock, pull_request, view.merge.clone())?;
        let mut waited = result(outcome, &view, checks, base);
        if outcome == Outcome::Failed {
            waited.failures = failures(gh, &waited.checks);
        }
        break waited;
    };
    activity(&activity_text(&waited));
    Ok(waited)
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
            "  Head commit now: {}. Watch the new commit with `factory pr wait` again.\n",
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

#[cfg(test)]
#[path = "pr_wait_tests.rs"]
mod tests;
