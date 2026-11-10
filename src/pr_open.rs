//! `factory pr open`: pushes a run's branch and opens its pull request, or finds the one that is open.

use crate::{
    inspect::shown,
    pr_wait::{problem, GhError, GhOutput, ToolProblem},
    run_begin::{first_line, git, last_line, valid_branch_name},
};
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

/// Runs `gh` in a folder, so that it finds the repository from the git remotes there. Tests answer from a
/// script.
pub(crate) trait Gh {
    fn run(&self, dir: &Path, args: &[&str]) -> Result<GhOutput, GhError>;

    fn az(&self, _dir: &Path, _args: &[&str]) -> Result<GhOutput, GhError> {
        Err(GhError::NotInstalled)
    }

    /// Runs git in a folder. Tests override this to keep the user's own git settings out.
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        git(dir, args)
    }
}

/// The real `gh`.
pub(crate) struct RealGh;

impl Gh for RealGh {
    fn az(&self, dir: &Path, args: &[&str]) -> Result<GhOutput, GhError> {
        let output = Command::new(crate::doctor::az_program())
            .current_dir(dir)
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

    fn run(&self, dir: &Path, args: &[&str]) -> Result<GhOutput, GhError> {
        let output = Command::new("gh")
            .current_dir(dir)
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

/// The factory server, as far as this command needs it.
pub(crate) trait RunApi {
    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String>;
}

/// What the command needs to know about the run.
#[derive(Clone, Debug)]
pub(crate) struct RunInfo {
    pub(crate) id: String,
    pub(crate) worktree: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) stage: String,
    /// `running`, `blocked` or `done`. Only a running run is updated.
    pub(crate) status: String,
}

impl RunInfo {
    pub(crate) fn running(&self) -> bool {
        self.status == "running"
    }
}

/// The longest title and body file that are accepted.
const MAX_TITLE_CHARS: usize = 256;
const MAX_BODY_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub(crate) title: String,
    pub(crate) body_file: PathBuf,
    pub(crate) base: Option<String>,
    pub(crate) json: bool,
}

/// `--title <title> --body-file <path> [--base <branch>] [--json]`, each at most once.
pub(crate) fn parse_options(options: &[&str]) -> Result<Options, String> {
    let (mut title, mut body, mut base, mut json) = (None, None, None, false);
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .copied()
                .ok_or_else(|| format!("{flag} needs a value."))
        };
        match *arg {
            "--json" if !json => json = true,
            "--title" if title.is_none() => title = Some(value("--title")?.to_string()),
            "--body-file" if body.is_none() => body = Some(value("--body-file")?.to_string()),
            "--base" if base.is_none() => {
                let name = value("--base")?;
                if !valid_branch_name(name) {
                    return Err(format!("{name:?} is not a usable branch name for --base."));
                }
                base = Some(name.to_string());
            }
            "--json" | "--title" | "--body-file" | "--base" => {
                return Err(format!("{arg} was given twice."))
            }
            other => return Err(format!("Unexpected option {other:?}.")),
        }
    }
    let title = title.ok_or("--title is needed.")?;
    let body = body.ok_or("--body-file is needed.")?;
    if title.trim().is_empty()
        || title.chars().count() > MAX_TITLE_CHARS
        || title.chars().any(char::is_control)
    {
        return Err(format!(
            "The title must be one line of 1 to {MAX_TITLE_CHARS} characters."
        ));
    }
    let metadata = std::fs::metadata(&body)
        .map_err(|error| format!("The body file {body:?} cannot be read: {error}."))?;
    if !metadata.is_file() || metadata.len() > MAX_BODY_BYTES {
        return Err(format!(
            "The body file {body:?} must be a file of at most {} KiB.",
            MAX_BODY_BYTES / 1024
        ));
    }
    let body_file = std::fs::canonicalize(&body)
        .map_err(|error| format!("The body file {body:?} cannot be read: {error}."))?;
    Ok(Options {
        title,
        body_file,
        base,
        json,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Opened {
    pub(crate) pull_request: String,
    /// Whether this call opened it; `false` when it was open already.
    pub(crate) created: bool,
    pub(crate) branch: String,
    pub(crate) base: String,
    /// The commit that was pushed.
    pub(crate) commit: String,
    /// Whether the run now records the pull request; `false` for a run that is not running.
    pub(crate) recorded: bool,
}

fn run_gh(gh: &dyn Gh, dir: &Path, args: &[&str]) -> Result<GhOutput, ToolProblem> {
    gh.run(dir, args).map_err(|error| match error {
        GhError::NotInstalled => ToolProblem(
            "The GitHub CLI (`gh`) is not installed or not on PATH. Install it from https://cli.github.com, run `gh auth login`, and try again."
                .into(),
        ),
        GhError::Other(text) => ToolProblem(format!("Could not run `gh`: {text}")),
    })
}

/// A rejected or failed push as a message that says what to do.
fn push_problem(branch: &str, error: &str) -> ToolProblem {
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
        return ToolProblem(format!(
            "git is not allowed to push to origin. Sign in (for example `gh auth login`, or set up your SSH key or credential helper) and try again. ({detail})"
        ));
    }
    if lower.contains("could not resolve host") || lower.contains("unable to access") {
        return ToolProblem(format!(
            "Could not reach origin. Check your network connection and try again. ({detail})"
        ));
    }
    ToolProblem(format!("Pushing {branch} to origin failed: {detail}"))
}

/// An open pull request of the branch.
struct Existing {
    url: String,
    /// Its base branch, when `gh` said.
    base: Option<String>,
}

/// The first open pull request of `branch` in this repository. `gh pr list --head` matches by branch name
/// only, so a pull request from a fork that happens to use the same name is skipped.
fn open_pull_request(
    gh: &dyn Gh,
    dir: &Path,
    branch: &str,
) -> Result<Option<Existing>, ToolProblem> {
    let args = [
        "pr",
        "list",
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "url,baseRefName,isCrossRepository",
    ];
    let output = run_gh(gh, dir, &args)?;
    if output.code != Some(0) {
        return Err(problem("pr list", &output));
    }
    let list: Vec<serde_json::Value> =
        serde_json::from_str(output.stdout.trim()).map_err(|error| {
            ToolProblem(format!(
                "`gh pr list` gave output that is not JSON: {error}"
            ))
        })?;
    let Some(item) = list
        .iter()
        .find(|item| item["isCrossRepository"].as_bool() != Some(true))
    else {
        return Ok(None);
    };
    let url = item["url"].as_str().ok_or_else(|| {
        ToolProblem(
            "`gh pr list` gave an open pull request without a URL, so it was not reused.".into(),
        )
    })?;
    Ok(Some(Existing {
        url: url.to_string(),
        base: item["baseRefName"]
            .as_str()
            .filter(|base| !base.is_empty())
            .map(str::to_string),
    }))
}

/// The pull request host a repository uses, as `factory pr open` and `factory doctor` see it.
pub(crate) enum Host {
    GitHub,
    Azure(AzureRepo),
    /// Neither the origin URL nor `factory.prProvider` names a supported host.
    Unknown,
}

/// The origin URL and the host it belongs to.
pub(crate) struct Origin {
    pub(crate) url: String,
    pub(crate) host: Host,
}

/// Works out the pull request host from origin and the `factory.prProvider` override. `git` answers a
/// read-only git question in the repository, or `None` when git fails. A missing origin, an unknown
/// override or an Azure override that does not match origin is an error.
pub(crate) fn detect_host(git: &dyn Fn(&[&str]) -> Option<String>) -> Result<Origin, ToolProblem> {
    let remote = git(&["remote", "get-url", "origin"]).ok_or_else(|| {
        ToolProblem("Cannot identify the repository provider: origin is not configured.".into())
    })?;
    let provider = git(&["config", "--get", "factory.prProvider"]);
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
            let org = get("factory.azureOrganization")?;
            let project = get("factory.azureProject")?;
            let repo = get("factory.azureRepository")?;
            let url = format!(
                "https://dev.azure.com/{}/{}/_git/{}",
                AzureRepo::encoded(&org),
                AzureRepo::encoded(&project),
                AzureRepo::encoded(&repo)
            );
            let selected = AzureRepo::from_remote(&url)?;
            match AzureRepo::from_remote(&remote)? {
                Some(actual) if Some(&actual) != selected.as_ref() => {
                    return Err(ToolProblem(
                        "Azure provider configuration does not match origin.".into(),
                    ))
                }
                None if get("factory.azureOrigin")? != remote => {
                    return Err(ToolProblem(
                        "Azure provider configuration does not match origin.".into(),
                    ))
                }
                _ => {}
            }
            selected
        }
        Some(_) => {
            return Err(ToolProblem(
                "Unknown factory.prProvider; choose github or azure.".into(),
            ))
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

/// Azure DevOps Services repository identified by its origin URL.
#[derive(PartialEq, Eq)]
pub(crate) struct AzureRepo {
    pub(crate) organization: String,
    pub(crate) project: String,
    pub(crate) repository: String,
}

impl AzureRepo {
    fn from_remote(remote: &str) -> Result<Option<Self>, ToolProblem> {
        let remote = remote.trim_end_matches('/');
        // Azure HTTPS remotes commonly include a username; it never belongs in output or API calls.
        let normalized = if let Some(rest) = remote.strip_prefix("https://") {
            if let Some((authority, path)) = rest.split_once('/') {
                let host = authority.rsplit('@').next().unwrap_or(authority);
                if host == "dev.azure.com" || host.ends_with(".visualstudio.com") {
                    format!("https://{host}/{path}")
                } else {
                    remote.to_string()
                }
            } else {
                remote.to_string()
            }
        } else {
            remote.to_string()
        };
        let remote = normalized.as_str();
        let parts: Vec<&str> = if let Some(path) = remote.strip_prefix("https://dev.azure.com/") {
            let p: Vec<_> = path.split('/').collect();
            if p.len() != 4 || p[2] != "_git" {
                return Err(ToolProblem("Invalid Azure Repos origin URL.".into()));
            }
            vec![p[0], p[1], p[3]]
        } else if let Some(path) = remote
            .strip_prefix("git@ssh.dev.azure.com:v3/")
            .or_else(|| remote.strip_prefix("ssh://git@ssh.dev.azure.com/v3/"))
            .or_else(|| remote.strip_prefix("ssh://git@ssh.dev.azure.com:22/v3/"))
        {
            path.split('/').collect()
        } else if let Some(path) = remote.strip_prefix("https://") {
            let Some((host, path)) = path.split_once('/') else {
                return Ok(None);
            };
            let Some(org) = host.strip_suffix(".visualstudio.com") else {
                return Ok(None);
            };
            let mut p: Vec<_> = path.split('/').collect();
            if p.first() == Some(&"DefaultCollection") {
                p.remove(0);
            }
            if p.len() != 3 || p[1] != "_git" {
                return Err(ToolProblem("Invalid Azure Repos origin URL.".into()));
            }
            vec![org, p[0], p[2]]
        } else {
            return Ok(None);
        };
        if parts.len() != 3 {
            return Err(ToolProblem("Invalid Azure Repos origin URL.".into()));
        }
        let decode = |part: &str| -> Result<String, ToolProblem> {
            let decoded = percent_encoding::percent_decode_str(part)
                .decode_utf8()
                .map_err(|_| ToolProblem("Invalid Azure Repos origin URL.".into()))?
                .into_owned();
            if decoded.is_empty()
                || decoded.starts_with('-')
                || decoded == "."
                || decoded == ".."
                || decoded
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\' | '?' | '#' | '@' | '%'))
            {
                return Err(ToolProblem("Invalid Azure Repos origin URL.".into()));
            }
            Ok(decoded)
        };
        Ok(Some(Self {
            organization: decode(parts[0])?,
            project: decode(parts[1])?,
            repository: decode(parts[2])?,
        }))
    }
    fn encoded(value: &str) -> String {
        encode_segment(value)
    }
    fn org_url(&self) -> String {
        format!(
            "https://dev.azure.com/{}",
            Self::encoded(&self.organization)
        )
    }
    fn pr_url(&self, id: u64) -> String {
        format!(
            "{}/{}/_git/{}/pullrequest/{id}",
            self.org_url(),
            Self::encoded(&self.project),
            Self::encoded(&self.repository)
        )
    }
}

fn az_json(gh: &dyn Gh, dir: &Path, args: &[&str]) -> Result<serde_json::Value, ToolProblem> {
    let result = gh.az(dir, args).map_err(|error| match error {
        GhError::NotInstalled => ToolProblem("Azure CLI (`az`) is not installed. Install it and the azure-devops extension, then sign in.".into()),
        GhError::Other(_) => ToolProblem("Could not start Azure CLI. Check its installation.".into()),
    })?;
    if result.code != Some(0) {
        // CLI output can contain credentials or remote URLs; do not include it in diagnostics.
        return Err(ToolProblem("Azure CLI request failed. Check the azure-devops extension, authentication, and repository access.".into()));
    }
    serde_json::from_str(&result.stdout)
        .map_err(|_| ToolProblem("Azure CLI returned invalid JSON.".into()))
}

fn azure_existing(
    gh: &dyn Gh,
    dir: &Path,
    repo: &AzureRepo,
    branch: &str,
) -> Result<Option<Existing>, ToolProblem> {
    let org = repo.org_url();
    let data = az_json(
        gh,
        dir,
        &[
            "repos",
            "pr",
            "list",
            "--organization",
            &org,
            "--project",
            &repo.project,
            "--repository",
            &repo.repository,
            "--source-branch",
            branch,
            "--status",
            "active",
            "--output",
            "json",
            "--detect",
            "false",
        ],
    )?;
    let list = data
        .as_array()
        .ok_or_else(|| ToolProblem("Azure PR list did not return an array.".into()))?;
    for item in list {
        // Cross-repository PRs with the same branch name must not be reused.
        if !item["forkSource"].is_null() {
            continue;
        }
        let id = item["pullRequestId"].as_u64().ok_or_else(|| {
            ToolProblem("Azure PR list returned a pull request without an ID.".into())
        })?;
        let base = item["targetRefName"]
            .as_str()
            .and_then(|s| s.strip_prefix("refs/heads/"))
            .map(str::to_string);
        return Ok(Some(Existing {
            url: repo.pr_url(id),
            base,
        }));
    }
    Ok(None)
}

struct PreparedPr {
    branch: String,
    base: String,
    commit: String,
}

fn open_azure(
    run: &RunInfo,
    options: &Options,
    gh: &dyn Gh,
    api: &dyn RunApi,
    dir: &Path,
    repo: &AzureRepo,
    prepared: PreparedPr,
) -> Result<Opened, ToolProblem> {
    let PreparedPr {
        branch,
        base,
        commit,
    } = prepared;
    let existing = azure_existing(gh, dir, repo, &branch)?;
    let description = if existing.is_none() {
        let text = std::fs::read_to_string(&options.body_file)
            .map_err(|_| ToolProblem("The pull request body could not be read as UTF-8.".into()))?;
        if text.chars().count() > 4000 || text.contains('\0') {
            return Err(ToolProblem("Azure pull request descriptions must be at most 4000 characters and contain no NUL bytes.".into()));
        }
        Some(format!("--description={text}"))
    } else {
        None
    };
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    gh.git(dir, &["push", "-u", "origin", &refspec]).map_err(|_| {
        ToolProblem("Git could not push to the Azure repository. Check Git authentication and branch permissions.".into())
    })?;
    if let Some(found) = existing {
        return finish(
            run,
            api,
            branch,
            found.base.unwrap_or(base),
            commit,
            found.url,
            false,
        );
    }
    let org = repo.org_url();
    let Some(description) = description else {
        return Err(ToolProblem(
            "The new Azure pull request has no description.".into(),
        ));
    };
    let result = az_json(
        gh,
        dir,
        &[
            "repos",
            "pr",
            "create",
            "--organization",
            &org,
            "--project",
            &repo.project,
            "--repository",
            &repo.repository,
            "--source-branch",
            &branch,
            "--target-branch",
            &base,
            &format!("--title={}", options.title),
            &description,
            "--output",
            "json",
            "--detect",
            "false",
        ],
    );
    let data = match result {
        Ok(data) => data,
        Err(error) => {
            return match azure_existing(gh, dir, repo, &branch) {
                Ok(Some(found)) => finish(
                    run,
                    api,
                    branch,
                    found.base.unwrap_or(base),
                    commit,
                    found.url,
                    false,
                ),
                _ => Err(error),
            }
        }
    };
    let id = data["pullRequestId"].as_u64().ok_or_else(|| ToolProblem("Azure PR creation returned no pull request ID; check for an existing PR before retrying.".into()))?;
    finish(run, api, branch, base, commit, repo.pr_url(id), true)
}

/// Pushes the run's branch and opens its pull request, or finds the open one, and records its URL on the run.
/// Nothing is pushed unless the checks pass and `gh` can see the repository. It never forces a push.
pub(crate) fn open(
    run: &RunInfo,
    options: &Options,
    gh: &dyn Gh,
    api: &dyn RunApi,
) -> Result<Opened, ToolProblem> {
    let worktree = run.worktree.as_deref().ok_or_else(|| {
        ToolProblem(format!(
            "Run {} has no worktree recorded. Record it with `factory run update` (--worktree and --branch), or create the workspace with `factory run begin`.",
            run.id
        ))
    })?;
    let dir = Path::new(worktree);
    if !dir.is_dir() {
        return Err(ToolProblem(format!(
            "The recorded worktree {} does not exist.",
            shown(worktree)
        )));
    }
    let Origin { url: remote, host } = detect_host(&|args| gh.git(dir, args).ok())?;
    let azure = match host {
        Host::Azure(repo) => Some(repo),
        Host::GitHub => None,
        Host::Unknown => return Err(ToolProblem("Cannot identify the repository provider from origin; set git config factory.prProvider github for a GitHub Enterprise repository.".into())),
    };
    if let Some(selected) = &azure {
        let push_urls = gh
            .git(dir, &["remote", "get-url", "--push", "--all", "origin"])
            .map_err(|_| ToolProblem("Cannot read the push destination of origin.".into()))?;
        if push_urls.is_empty()
            || push_urls
                .lines()
                .any(|url| match AzureRepo::from_remote(url) {
                    Ok(Some(repo)) => &repo != selected,
                    Ok(None) => url != remote,
                    Err(_) => true,
                })
        {
            return Err(ToolProblem(
                "The push destination of origin does not match the Azure repository.".into(),
            ));
        }
    }
    if azure.is_none() {
        run_gh(gh, dir, &["--version"])?;
    }

    let branch = gh
        .git(dir, &["branch", "--show-current"])
        .map_err(|error| {
            ToolProblem(format!(
                "{} could not be read: {}",
                shown(worktree),
                shown(first_line(&error))
            ))
        })?;
    if branch.is_empty() {
        return Err(ToolProblem(format!(
            "The worktree {} is not on a branch (a detached HEAD).",
            shown(worktree)
        )));
    }
    // A branch name that git could read as an option or a refspec (`+main` forces an update of main) is
    // never handed to git.
    if !valid_branch_name(&branch) {
        return Err(ToolProblem(format!(
            "The branch name {} is one this command will not push: it could be read as a git option or a forced refspec. Rename the branch (`git branch -m`) and try again.",
            shown(&branch)
        )));
    }
    if let Some(recorded) = &run.branch {
        if *recorded != branch {
            return Err(ToolProblem(format!(
                "The worktree is on {} but the run records {}. Switch it back, or record the right branch with `factory run update`.",
                shown(&branch),
                shown(recorded)
            )));
        }
    }
    let base = match &options.base {
        Some(base) => base.clone(),
        None => gh
            .git(
                dir,
                &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
            )
            .ok()
            .and_then(|head| head.strip_prefix("origin/").map(str::to_string))
            .ok_or_else(|| {
                ToolProblem(
                    "origin/HEAD is not set, so the base branch is not known. Pass it with --base."
                        .into(),
                )
            })?,
    };
    if branch == base {
        return Err(ToolProblem(format!(
            "{} is the base branch. A pull request needs a feature branch.",
            shown(&branch)
        )));
    }
    let status = gh.git(dir, &["status", "--porcelain"]).map_err(|error| {
        ToolProblem(format!(
            "The status of {} could not be read: {}",
            shown(worktree),
            shown(first_line(&error))
        ))
    })?;
    if !status.is_empty() {
        return Err(ToolProblem(format!(
            "The worktree has uncommitted changes ({} paths). Commit them, then try again.",
            status.lines().count()
        )));
    }
    let ahead = gh
        .git(
            dir,
            &["rev-list", "--count", &format!("refs/remotes/origin/{base}..HEAD")],
        )
        .map_err(|_| {
            ToolProblem(format!(
                "origin/{} is not known in this repository. Run `git fetch origin {}` and try again.",
                shown(&base),
                shown(&base)
            ))
        })?;
    if ahead == "0" {
        return Err(ToolProblem(format!(
            "{} has no commits beyond origin/{}, so there is nothing to open a pull request for.",
            shown(&branch),
            shown(&base)
        )));
    }
    let commit = gh.git(dir, &["rev-parse", "HEAD"]).map_err(|error| {
        ToolProblem(format!(
            "HEAD could not be read: {}",
            shown(first_line(&error))
        ))
    })?;

    // Look for the pull request before pushing: it also shows whether `gh` is signed in.
    if let Some(azure) = azure {
        return open_azure(
            run,
            options,
            gh,
            api,
            dir,
            &azure,
            PreparedPr {
                branch,
                base,
                commit,
            },
        );
    }
    let existing = open_pull_request(gh, dir, &branch)?;
    // The branch is pushed by its full name on both sides, so that no name is read as a refspec.
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    gh.git(dir, &["push", "-u", "origin", &refspec])
        .map_err(|error| push_problem(&branch, &error))?;

    let (pull_request, created, base) = match existing {
        Some(found) => (found.url, false, found.base.unwrap_or(base)),
        None => {
            let body = options.body_file.display().to_string();
            let output = run_gh(
                gh,
                dir,
                &[
                    "pr",
                    "create",
                    "--base",
                    &base,
                    "--head",
                    &branch,
                    "--title",
                    &options.title,
                    "--body-file",
                    &body,
                ],
            )?;
            if output.code != Some(0) {
                // Someone may have opened it between the look and now: use theirs.
                return match open_pull_request(gh, dir, &branch) {
                    Ok(Some(found)) => finish(
                        run,
                        api,
                        branch,
                        found.base.unwrap_or(base),
                        commit,
                        found.url,
                        false,
                    ),
                    _ => Err(problem("pr create", &output)),
                };
            }
            let url = output
                .stdout
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| line.starts_with("https://") || line.starts_with("http://"))
                .ok_or_else(|| {
                    ToolProblem(format!(
                        "`gh pr create` did not print the pull request's URL: {}",
                        shown(first_line(&output.stdout))
                    ))
                })?
                .to_string();
            (url, true, base)
        }
    };
    finish(run, api, branch, base, commit, pull_request, created)
}

/// Records the pull request on a running run and returns the result.
fn finish(
    run: &RunInfo,
    api: &dyn RunApi,
    branch: String,
    base: String,
    commit: String,
    pull_request: String,
    created: bool,
) -> Result<Opened, ToolProblem> {
    if run.running() {
        api.update(
            &run.id,
            &run.stage,
            "running",
            &["--pr", &pull_request, "--activity", "Opened pull request"],
        )
        .map_err(|error| {
            ToolProblem(format!(
                "The pull request is open at {}, but the run could not be updated: {error}. Run the command again; it finds the pull request and records it.",
                shown(&pull_request)
            ))
        })?;
    }
    Ok(Opened {
        pull_request,
        created,
        branch,
        base,
        commit,
        recorded: run.running(),
    })
}

/// The result as text.
pub(crate) fn render(opened: &Opened, run: &RunInfo) -> String {
    let mut text = format!(
        "{}: {}\n  Branch: {}\n  Base: {}\n  Pushed commit: {}\n",
        if opened.created {
            "Pull request opened"
        } else {
            "Pull request already open"
        },
        shown(&opened.pull_request),
        shown(&opened.branch),
        shown(&opened.base),
        shown(&opened.commit.chars().take(7).collect::<String>())
    );
    if !opened.recorded {
        let summary = if run.stage == "finish" && run.status == "done" {
            " --summary finish=<what shipped>"
        } else {
            ""
        };
        text.push_str(&format!(
            "  The run was not updated (it is {}). Record the pull request with `factory run update {} {} {}{} --pr {}`.\n",
            shown(&run.status),
            shown(&run.id),
            shown(&run.stage),
            shown(&run.status),
            summary,
            shown(&opened.pull_request)
        ));
    }
    text
}

#[cfg(test)]
#[path = "pr_open_tests.rs"]
mod tests;
