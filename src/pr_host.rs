//! Pull request host: reads pull requests on GitHub (with `gh`) and Azure Repos (with `az`) behind one
//! interface, so callers never ask which host a pull request lives on. It also works out which host a
//! repository uses from its origin remote.

mod azure;
mod github;

pub(crate) use azure::{AzurePr, AzureRepo, az_json, az_parsed, required};
pub(crate) use github::{problem, run_gh, started};

use crate::pr_state::PrState;
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

/// Reads pull requests on their host, whichever host that is.
pub(crate) trait PrHost {
    /// The state of the pull request at `url`.
    fn state(&self, url: &str) -> Result<PrState, ToolProblem>;

    /// The head of the pull request at `url` and its state, read in one call to the host.
    fn head(&self, url: &str) -> Result<Head, ToolProblem>;
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
    calls: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeHost {
    /// Answers each call with the next answer, whatever the pull request; the last answer repeats. An error
    /// answer is the problem's message.
    pub(crate) fn new(answers: Vec<Result<Head, String>>) -> Self {
        Self {
            answers: std::sync::Mutex::new(answers),
            calls: std::sync::Mutex::new(Vec::new()),
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

    /// The calls so far, as `state <url>` or `head <url>`.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the call log is not poisoned")
            .clone()
    }

    fn answer(&self, call: &str, url: &str) -> Result<Head, ToolProblem> {
        self.calls
            .lock()
            .expect("the call log is not poisoned")
            .push(format!("{call} {url}"));
        let mut answers = self.answers.lock().expect("the answers are not poisoned");
        let answer = if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers
                .first()
                .expect("the fake host has an answer")
                .clone()
        };
        answer.map_err(ToolProblem)
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
}

#[cfg(test)]
mod tests;
