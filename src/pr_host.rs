//! Pull request host: reads pull requests on GitHub (with `gh`) and Azure Repos (with `az`) behind one
//! interface, so callers never ask which host a pull request lives on. It also works out which host a
//! repository uses from its origin remote.

mod azure;
mod github;

pub(crate) use azure::{AzurePr, AzureRepo, az_json, required};
pub(crate) use github::{problem, run_gh};

use crate::{inspect::shown, pr_state::PrState, run_begin::last_line};
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
        let mut command = Command::new(crate::doctor::az_program());
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
    fn state(&self, url: &str) -> Result<PrState, ToolProblem>;

    /// The head of the pull request at `url` and its state, read in one call to the host.
    fn head(&self, url: &str) -> Result<Head, ToolProblem>;

    /// The open pull request from `branch` in origin's repository, if there is one.
    ///
    /// A pull request from a fork that happens to use the same branch name is skipped. `dir` is the
    /// worktree, where `gh` finds the repository from the git remotes.
    fn find_open(
        &self,
        dir: &Path,
        origin: &Origin,
        branch: &str,
    ) -> Result<Option<OpenPr>, ToolProblem>;

    /// Checks that the host will take `pr`, so that a refusal comes before anything is pushed.
    fn check_new(&self, origin: &Origin, pr: &NewPr) -> Result<(), ToolProblem>;

    /// Opens `pr` in origin's repository and returns its URL. `dir` is the worktree.
    fn create(&self, dir: &Path, origin: &Origin, pr: &NewPr) -> Result<String, ToolProblem>;
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
                "The pull request is not an http(s) URL: {url}"
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
        match &origin.host {
            Host::GitHub => github::find_open(&self.tools, dir, branch),
            Host::Azure(repo) => azure::find_open(&self.tools, repo, branch),
            Host::Unknown => Err(unknown_origin()),
        }
    }

    fn check_new(&self, origin: &Origin, pr: &NewPr) -> Result<(), ToolProblem> {
        match &origin.host {
            Host::GitHub => Ok(()),
            Host::Azure(_) => azure::description(pr).map(drop),
            Host::Unknown => Err(unknown_origin()),
        }
    }

    fn create(&self, dir: &Path, origin: &Origin, pr: &NewPr) -> Result<String, ToolProblem> {
        match &origin.host {
            Host::GitHub => github::create(&self.tools, dir, pr),
            Host::Azure(repo) => azure::create(&self.tools, repo, pr),
            Host::Unknown => Err(unknown_origin()),
        }
    }
}

fn unsupported() -> ToolProblem {
    ToolProblem("Only GitHub and Azure Repos pull requests can be read.".into())
}

/// Whether `url` is an Azure DevOps Services pull request URL, in its canonical or legacy form.
pub(crate) fn is_azure_pr_url(url: &str) -> bool {
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

/// Works out the pull request host from origin and the `jidoka.prProvider` override. `git` answers a
/// read-only git question in the repository, or `None` when git fails. A missing origin, an unknown
/// override or an Azure override that does not match origin is an error.
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
        None if provider.as_deref() == Some("github") || is_github_remote(&remote) => Host::GitHub,
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

fn is_github_remote(remote: &str) -> bool {
    if remote.starts_with("git@github.com:") {
        return true;
    }
    let rest = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
        .or_else(|| remote.strip_prefix("ssh://"));
    rest.and_then(|url| url.split_once('/'))
        .is_some_and(|(authority, _)| {
            let host = authority.rsplit('@').next().unwrap_or(authority);
            host.eq_ignore_ascii_case("github.com")
                || host.eq_ignore_ascii_case("github.com:22")
                || host.eq_ignore_ascii_case("github.com:443")
        })
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

    /// The calls so far, as `state <url>`, `head <url>`, `find_open <branch>`, `check_new <branch>` or
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
}

#[cfg(test)]
mod tests;
