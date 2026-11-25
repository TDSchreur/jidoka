//! Pull request host: reads, opens and watches pull requests on GitHub and Azure Repos.
//!
//! GitHub is reached with `gh` and Azure Repos with `az`, behind one interface, so callers never ask which
//! host a pull request lives on. It also works out which host a repository uses from its origin remote.

mod azure;
mod github;
mod watch;

pub(crate) use azure::AzureRepo;
pub(crate) use github::GitHubRepo;

use crate::{
    git::last_line,
    pr::state::PrState,
    pr::wait::{CheckResult, Clock, Failure, Options, Waited},
    run::inspect::shown,
};
use std::{io, path::Path, process::Command};

/// A problem with the tools or the pull request, not an outcome. The message says what to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ToolProblem(pub(crate) String);

/// What a command line tool printed.
#[derive(Clone, Debug)]
pub(crate) struct ToolOutput {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Why a command line tool could not be started.
#[derive(Debug)]
pub(crate) enum ToolError {
    NotInstalled,
    Other(String),
}

/// Starts the hosts' command line tools. The real implementation starts programs; adapter tests answer
/// from a script.
pub(crate) trait Tools {
    /// Runs `gh` with `args`, in `dir` when given, so that it finds the repository from the git remotes there.
    fn gh(&self, dir: Option<&Path>, args: &[&str]) -> Result<ToolOutput, ToolError>;

    /// Runs the Azure CLI with `args`.
    fn az(&self, args: &[&str]) -> Result<ToolOutput, ToolError>;
}

impl<T: Tools + ?Sized> Tools for &T {
    fn gh(&self, dir: Option<&Path>, args: &[&str]) -> Result<ToolOutput, ToolError> {
        (**self).gh(dir, args)
    }

    fn az(&self, args: &[&str]) -> Result<ToolOutput, ToolError> {
        (**self).az(args)
    }
}

/// The real `gh` and `az`.
pub(crate) struct RealTools;

/// The hosts as the CLI and the dashboard reach them: through the real `gh` and `az`. The one place they are built.
pub(crate) fn real_hosts() -> Hosts<RealTools> {
    Hosts::new(RealTools)
}

impl Tools for RealTools {
    fn gh(&self, dir: Option<&Path>, args: &[&str]) -> Result<ToolOutput, ToolError> {
        let mut command = Command::new("gh");
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        command
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1");
        output(&mut command)
    }

    fn az(&self, args: &[&str]) -> Result<ToolOutput, ToolError> {
        // Windows installs `az` as a `.cmd` launcher, which only starts by the path `az_program` finds.
        let mut command = Command::new(crate::program::az_program());
        command
            .args(args)
            .env("AZURE_CORE_ONLY_SHOW_ERRORS", "true")
            .env("AZURE_EXTENSION_USE_DYNAMIC_INSTALL", "no");
        output(&mut command)
    }
}

fn output(command: &mut Command) -> Result<ToolOutput, ToolError> {
    let output = command.output().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ToolError::NotInstalled
        } else {
            ToolError::Other(error.to_string())
        }
    })?;
    Ok(ToolOutput {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Reads and opens pull requests on their host, whichever host that is.
pub(crate) trait PrHost {
    /// The state of the pull request at `url`.
    ///
    /// # Errors
    ///
    /// Returns a problem that says what to do when `url` is not a supported pull request URL or the host
    /// cannot be read.
    fn state(&self, url: &str) -> Result<PrState, ToolProblem>;

    /// The head of the pull request at `url` and its state, read in one call to the host.
    ///
    /// # Errors
    ///
    /// Returns a problem when `url` is not a supported pull request URL, the host cannot be read, or it
    /// reports no branch or no valid commit.
    fn head(&self, url: &str) -> Result<Head, ToolProblem>;

    /// The open pull request from `branch` in origin's repository, if there is one.
    ///
    /// A pull request from a fork that happens to use the same branch name is skipped. `dir` is the
    /// worktree, where `gh` finds the repository from the git remotes.
    ///
    /// # Errors
    ///
    /// Returns a problem when origin's host is unknown or the host cannot list its pull requests.
    fn find_open(
        &self,
        dir: &Path,
        origin: &Origin,
        branch: &str,
    ) -> Result<Option<OpenPr>, ToolProblem>;

    /// Checks that the host will take `pr`, so that a refusal comes before anything is pushed.
    ///
    /// # Errors
    ///
    /// Returns a problem when origin's host is unknown or will not take the title or the description.
    fn check_new(&self, origin: &Origin, pr: &NewPr) -> Result<(), ToolProblem>;

    /// Opens `pr` in origin's repository and returns its URL. `dir` is the worktree.
    ///
    /// # Errors
    ///
    /// Returns a problem when origin's host is unknown or the host does not open the pull request.
    fn create(&self, dir: &Path, origin: &Origin, pr: &NewPr) -> Result<String, ToolProblem>;

    /// Watches the checks of the pull request at `url` until they have ended.
    ///
    /// The checks are those of its current head commit. Returns the CI outcome with the state the pull
    /// request had then. `activity` is told what the run is doing, and `progress` how far the checks are.
    /// It only reads: it never merges, pushes or updates the branch.
    ///
    /// # Errors
    ///
    /// Returns a problem when `url` is not a supported pull request URL or the host cannot be read.
    fn watch(
        &self,
        url: &str,
        options: &Options,
        clock: &dyn Clock,
        activity: &mut dyn FnMut(&str),
        progress: &mut dyn FnMut(&str),
    ) -> Result<Waited, ToolProblem>;
}

/// An open pull request of a branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenPr {
    pub(crate) url: String,
    /// Its base branch, when the host said.
    pub(crate) base: Option<String>,
}

/// A pull request to open: from `branch` into `base`, with the description in `body_file`.
pub(crate) struct NewPr<'a> {
    pub(crate) branch: &'a str,
    pub(crate) base: &'a str,
    pub(crate) title: &'a str,
    pub(crate) body_file: &'a Path,
}

/// The head of a pull request: its last commit and the branch it comes from, with its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Head {
    pub(crate) state: PrState,
    /// The commit, in lowercase hex.
    pub(crate) commit: String,
    pub(crate) branch: String,
}

/// What one host does for `Hosts`; each host has its own adapter.
trait Adapter {
    fn state(&self, tools: &dyn Tools, url: &str) -> Result<PrState, ToolProblem>;
    fn head(&self, tools: &dyn Tools, url: &str) -> Result<Head, ToolProblem>;

    /// One consistent look at the pull request for a watch of its checks.
    fn snapshot(&self, tools: &dyn Tools, url: &str) -> Result<watch::Snapshot, ToolProblem>;

    /// The open pull request from `branch` in the repository of `origin`; `dir` is the worktree.
    fn find_open(
        &self,
        tools: &dyn Tools,
        dir: &Path,
        origin: &Origin,
        branch: &str,
    ) -> Result<Option<OpenPr>, ToolProblem>;

    /// Checks that the host will take `pr`. By default it takes every pull request.
    fn check_new(&self, _pr: &NewPr) -> Result<(), ToolProblem> {
        Ok(())
    }

    /// Opens `pr` in the repository of `origin` and returns its URL; `dir` is the worktree.
    fn create(
        &self,
        tools: &dyn Tools,
        dir: &Path,
        origin: &Origin,
        pr: &NewPr,
    ) -> Result<String, ToolProblem>;

    /// The failed checks of a failed watch. A host that can fetch their logs adds the end of each log;
    /// by default there is none.
    fn failures(&self, _tools: &dyn Tools, checks: &[CheckResult]) -> Vec<Failure> {
        checks
            .iter()
            .filter(|check| check.bucket == "fail")
            .map(|check| Failure {
                name: check.name.clone(),
                link: check.link.clone(),
                excerpt: None,
            })
            .collect()
    }
}

/// Limit log lookups to three CI runs so a failing matrix does not delay the outcome.
const MAX_LOGS: usize = 3;
/// Keep the end of the failed log, where the cause is likely to be.
const LOG_LINES: usize = 40;
/// Cap the excerpt at 4 KiB so an unusually long line cannot flood the wait output.
const LOG_BYTES: usize = 4096;

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

/// The real pull request host: picks the GitHub or the Azure Repos adapter from the pull request's URL,
/// and starts their tools through `tools`.
pub(crate) struct Hosts<T> {
    tools: T,
}

impl<T: Tools> Hosts<T> {
    pub(crate) fn new(tools: T) -> Self {
        Self { tools }
    }

    /// The adapter of the host `url` belongs to: Azure Repos for an Azure DevOps Services URL, GitHub (or
    /// GitHub Enterprise) for a `/pull/<number>` URL.
    fn adapter(url: &str) -> Result<&'static dyn Adapter, ToolProblem> {
        // Only an absolute http(s) URL is handed to a tool, so a stored value can never become an option.
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(ToolProblem(format!(
                "The pull request is not an http(s) URL: {}",
                shown(url)
            )));
        }
        if is_azure_pr_url(url) {
            return Ok(&azure::Azure);
        }
        if github::pull_request(url).is_some() {
            return Ok(&github::GitHub);
        }
        Err(unsupported())
    }

    /// The adapter of the host origin belongs to.
    fn origin_adapter(origin: &Origin) -> Result<&'static dyn Adapter, ToolProblem> {
        match origin.host {
            Host::GitHub => Ok(&github::GitHub),
            Host::Azure(_) => Ok(&azure::Azure),
            Host::Unknown => Err(unknown_origin()),
        }
    }
}

impl<T: Tools> PrHost for Hosts<T> {
    fn state(&self, url: &str) -> Result<PrState, ToolProblem> {
        Self::adapter(url)?.state(&self.tools, url)
    }

    fn head(&self, url: &str) -> Result<Head, ToolProblem> {
        let head = Self::adapter(url)?.head(&self.tools, url)?;
        if head.branch.is_empty() {
            return Err(ToolProblem(
                "The host did not report the branch the pull request comes from.".into(),
            ));
        }
        let commit = &head.commit;
        if commit.len() < 4 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ToolProblem(
                "The host did not report a valid commit for the pull request.".into(),
            ));
        }
        Ok(Head {
            commit: commit.to_ascii_lowercase(),
            ..head
        })
    }

    fn find_open(
        &self,
        dir: &Path,
        origin: &Origin,
        branch: &str,
    ) -> Result<Option<OpenPr>, ToolProblem> {
        Self::origin_adapter(origin)?.find_open(&self.tools, dir, origin, branch)
    }

    fn check_new(&self, origin: &Origin, pr: &NewPr) -> Result<(), ToolProblem> {
        Self::origin_adapter(origin)?.check_new(pr)
    }

    fn create(&self, dir: &Path, origin: &Origin, pr: &NewPr) -> Result<String, ToolProblem> {
        Self::origin_adapter(origin)?.create(&self.tools, dir, origin, pr)
    }

    fn watch(
        &self,
        url: &str,
        options: &Options,
        clock: &dyn Clock,
        activity: &mut dyn FnMut(&str),
        progress: &mut dyn FnMut(&str),
    ) -> Result<Waited, ToolProblem> {
        let pr = watch::PullRequest {
            adapter: Self::adapter(url)?,
            tools: &self.tools,
            url,
        };
        watch::watch(&pr, options, clock, activity, progress)
    }
}

fn unsupported() -> ToolProblem {
    ToolProblem("Only GitHub and Azure Repos pull requests can be read.".into())
}

/// Whether `url` is an Azure DevOps Services pull request URL, in its canonical or legacy form.
fn is_azure_pr_url(url: &str) -> bool {
    url.starts_with("https://dev.azure.com/")
        || url
            .strip_prefix("https://")
            .and_then(|rest| rest.split('/').next())
            .is_some_and(|host| host.ends_with(".visualstudio.com"))
}

/// The pull request host a repository uses, as `jidoka pr open` and `jidoka doctor` see it.
pub(crate) enum Host {
    GitHub,
    Azure(AzureRepo),
    /// Neither the origin URL nor `jidoka.prProvider` names a supported host.
    Unknown,
}

/// The origin URL and the host it belongs to.
pub(crate) struct Origin {
    pub(crate) url: String,
    pub(crate) host: Host,
}

/// Works out the pull request host from origin and the `jidoka.prProvider` override.
///
/// `git` answers a read-only git question in the repository, or `None` when git fails.
///
/// # Errors
///
/// Returns a problem that says what to fix when origin is missing, the override is unknown, an Azure
/// override lacks its settings or does not match origin, or origin is not a valid Azure URL.
pub(crate) fn detect_host(git: &dyn Fn(&[&str]) -> Option<String>) -> Result<Origin, ToolProblem> {
    let remote = git(&["remote", "get-url", "origin"]).ok_or_else(|| {
        ToolProblem("Cannot identify the repository provider: origin is not configured.".into())
    })?;
    let provider = git(&["config", "--get", "jidoka.prProvider"]);
    let azure = match provider.as_deref() {
        None => AzureRepo::from_remote(&remote)?,
        Some("github") => None,
        Some("azure") => {
            let get = |key: &str| {
                git(&["config", "--get", key]).ok_or_else(|| {
                    ToolProblem(format!(
                        "Set git config {key} for the Azure provider override."
                    ))
                })
            };
            let org = get("jidoka.azureOrganization")?;
            let project = get("jidoka.azureProject")?;
            let repo = get("jidoka.azureRepository")?;
            let url = format!(
                "https://dev.azure.com/{}/{}/_git/{}",
                encode_segment(&org),
                encode_segment(&project),
                encode_segment(&repo)
            );
            let selected = AzureRepo::from_remote(&url)?;
            match AzureRepo::from_remote(&remote)? {
                Some(actual) if Some(&actual) != selected.as_ref() => {
                    return Err(ToolProblem(
                        "Azure provider configuration does not match origin.".into(),
                    ));
                }
                None if get("jidoka.azureOrigin")? != remote => {
                    return Err(ToolProblem(
                        "Azure provider configuration does not match origin.".into(),
                    ));
                }
                _ => {}
            }
            selected
        }
        Some(_) => {
            return Err(ToolProblem(
                "Unknown jidoka.prProvider; choose github or azure.".into(),
            ));
        }
    };
    let host = match azure {
        Some(repo) => Host::Azure(repo),
        None if provider.as_deref() == Some("github") || github::is_remote(&remote) => Host::GitHub,
        None => Host::Unknown,
    };
    Ok(Origin { url: remote, host })
}

fn unknown_origin() -> ToolProblem {
    ToolProblem("Cannot identify the repository provider from origin; set git config jidoka.prProvider github for a GitHub Enterprise repository.".into())
}

/// Checks, before anything is pushed, that pull requests can be opened from `origin`.
///
/// Its host must be known, and an Azure origin must push to the repository it names. `git` answers a
/// read-only git question in the repository, or `None` when git fails.
///
/// # Errors
///
/// Returns a problem that says what to fix when the host is unknown or the push destination differs.
pub(crate) fn check_origin(
    origin: &Origin,
    git: &dyn Fn(&[&str]) -> Option<String>,
) -> Result<(), ToolProblem> {
    let selected = match &origin.host {
        Host::GitHub => return Ok(()),
        Host::Unknown => return Err(unknown_origin()),
        Host::Azure(repo) => repo,
    };
    let push_urls = git(&["remote", "get-url", "--push", "--all", "origin"])
        .ok_or_else(|| ToolProblem("Cannot read the push destination of origin.".into()))?;
    let differs = |url: &str| match AzureRepo::from_remote(url) {
        Ok(Some(repo)) => &repo != selected,
        Ok(None) => url != origin.url,
        Err(_) => true,
    };
    if push_urls.is_empty() || push_urls.lines().any(differs) {
        return Err(ToolProblem(
            "The push destination of origin does not match the Azure repository.".into(),
        ));
    }
    Ok(())
}

/// A rejected or failed push of `branch` to `origin` as a message that says what to do.
///
/// Git's own output is not shown for an Azure origin, the same as Azure CLI output.
pub(crate) fn push_problem(origin: &Origin, branch: &str, error: &str) -> ToolProblem {
    if matches!(origin.host, Host::Azure(_)) {
        return ToolProblem("Git could not push to the Azure repository. Check Git authentication and branch permissions.".into());
    }
    let lower = error.to_lowercase();
    let detail = shown(last_line(error));
    if lower.contains("rejected")
        || lower.contains("non-fast-forward")
        || lower.contains("fetch first")
    {
        return ToolProblem(format!(
            "origin rejected the push of {branch}: it has commits this branch does not. Merge the remote branch or the base into it with a normal merge, never a force-push, and try again. ({detail})"
        ));
    }
    if lower.contains("authentication failed")
        || lower.contains("could not read username")
        || lower.contains("permission denied")
        || lower.contains("publickey")
    {
        // Git prints a failing credential helper's complaint before the final `fatal:` line, so show every line.
        let all_lines = error
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        return ToolProblem(format!(
            "git is not allowed to push to origin. Read git's output below first: a credential helper that crashes is not fixed by signing in. Otherwise sign in (for example `gh auth login`, or set up your SSH key or credential helper; `git config --show-origin --get-all credential.helper` lists the helpers git uses) and try again. ({})",
            shown(&all_lines)
        ));
    }
    if lower.contains("could not resolve host") || lower.contains("unable to access") {
        return ToolProblem(format!(
            "Could not reach origin. Check your network connection and try again. ({detail})"
        ));
    }
    ToolProblem(format!("Pushing {branch} to origin failed: {detail}"))
}

/// What is percent-encoded in an Azure URL path segment: everything but letters, digits and the unreserved
/// `-`, `.`, `_` and `~` of RFC 3986, so a name like `my-repo` appears as Azure itself shows it.
const URL_SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `value` percent-encoded as one segment of an Azure URL path.
pub(crate) fn encode_segment(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, URL_SEGMENT).to_string()
}

/// An in-memory pull request host for the tests of its callers.
#[cfg(test)]
pub(crate) struct FakeHost {
    answers: std::sync::Mutex<Vec<Result<Head, String>>>,
    /// The answers of `find_open`, in turn; the last one repeats.
    open: std::sync::Mutex<Vec<Option<OpenPr>>>,
    /// The URL `create` opens, or its problem.
    created: Result<String, String>,
    /// The CI outcome `watch` gives an open pull request.
    ci: crate::pr::wait::Outcome,
    calls: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeHost {
    /// Answers each call with the next answer, whatever the pull request; the last answer repeats. An error
    /// answer is the problem's message.
    pub(crate) fn new(answers: Vec<Result<Head, String>>) -> Self {
        Self {
            answers: std::sync::Mutex::new(answers),
            open: std::sync::Mutex::new(vec![None]),
            created: Err("The fake host opens no pull request.".into()),
            ci: crate::pr::wait::Outcome::Passed,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// No pull request is open, and creating one opens `url`.
    pub(crate) fn opening(url: &str) -> Self {
        Self {
            created: Ok(url.into()),
            ..Self::new(Vec::new())
        }
    }

    /// `open` is open already.
    pub(crate) fn with_open(open: OpenPr) -> Self {
        let host = Self::new(Vec::new());
        *host.open.lock().expect("the answers are not poisoned") = vec![Some(open)];
        host
    }

    /// No pull request is open, and creating one fails with `problem`.
    pub(crate) fn failing_create(problem: &str) -> Self {
        Self {
            created: Err(problem.into()),
            ..Self::new(Vec::new())
        }
    }

    /// Answers with `states` in turn, for a pull request from branch `feature` at commit `abc1234`.
    pub(crate) fn states(states: &[PrState]) -> Self {
        Self::new(
            states
                .iter()
                .map(|&state| {
                    Ok(Head {
                        state,
                        commit: "abc1234".into(),
                        branch: "feature".into(),
                    })
                })
                .collect(),
        )
    }

    /// `watch` gives an open pull request the CI outcome `ci` instead of `passed`.
    pub(crate) fn with_ci(self, ci: crate::pr::wait::Outcome) -> Self {
        Self { ci, ..self }
    }

    /// The calls so far, as `state <url>`, `head <url>`, `watch <url>`, `find_open <branch>`, `check_new <branch>` or
    /// `create <branch> into <base>: <title>`.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the call log is not poisoned")
            .clone()
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .expect("the call log is not poisoned")
            .push(call);
    }

    fn answer(&self, call: &str, url: &str) -> Result<Head, ToolProblem> {
        self.record(format!("{call} {url}"));
        next(&self.answers).map_err(ToolProblem)
    }
}

/// The next of `answers`; the last one repeats.
#[cfg(test)]
fn next<T: Clone>(answers: &std::sync::Mutex<Vec<T>>) -> T {
    let mut answers = answers.lock().expect("the answers are not poisoned");
    if answers.len() > 1 {
        answers.remove(0)
    } else {
        answers
            .first()
            .expect("the fake host has an answer")
            .clone()
    }
}

#[cfg(test)]
impl PrHost for FakeHost {
    fn state(&self, url: &str) -> Result<PrState, ToolProblem> {
        self.answer("state", url).map(|head| head.state)
    }

    fn head(&self, url: &str) -> Result<Head, ToolProblem> {
        self.answer("head", url)
    }

    fn find_open(
        &self,
        _dir: &Path,
        _origin: &Origin,
        branch: &str,
    ) -> Result<Option<OpenPr>, ToolProblem> {
        self.record(format!("find_open {branch}"));
        Ok(next(&self.open))
    }

    fn check_new(&self, _origin: &Origin, pr: &NewPr) -> Result<(), ToolProblem> {
        self.record(format!("check_new {}", pr.branch));
        Ok(())
    }

    fn create(&self, _dir: &Path, _origin: &Origin, pr: &NewPr) -> Result<String, ToolProblem> {
        self.record(format!(
            "create {} into {}: {}",
            pr.branch, pr.base, pr.title
        ));
        self.created.clone().map_err(ToolProblem)
    }

    /// Answers at once with the next answer's state and commit: `not-open` for a merged or closed pull
    /// request, else the CI outcome the fake was given.
    fn watch(
        &self,
        url: &str,
        _options: &Options,
        _clock: &dyn Clock,
        activity: &mut dyn FnMut(&str),
        _progress: &mut dyn FnMut(&str),
    ) -> Result<Waited, ToolProblem> {
        use crate::pr::wait::{BaseState, Outcome};
        let head = self.answer("watch", url)?;
        let outcome = match head.state {
            PrState::Open | PrState::Draft => self.ci,
            _ => Outcome::NotOpen,
        };
        let waited = Waited {
            outcome,
            exit_code: outcome.exit_code(),
            pull_request: url.into(),
            head: head.commit,
            new_head: None,
            base: BaseState::UpToDate,
            checks: Vec::new(),
            failures: Vec::new(),
            state: head.state,
        };
        activity(&crate::pr::wait::activity_text(&waited));
        Ok(waited)
    }
}

#[cfg(test)]
mod tests;
