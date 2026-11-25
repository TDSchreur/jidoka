//! The GitHub adapter: reads pull requests on GitHub and GitHub Enterprise with `gh`.

use super::{
    Adapter, Head, MAX_LOGS, NewPr, OpenPr, Origin, ToolError, ToolOutput, ToolProblem, Tools,
    excerpt, unsupported, watch::Snapshot,
};
use crate::{
    git::first_line,
    inspect::shown,
    pr_state::PrState,
    pr_wait::{BaseState, CheckResult, Failure},
};
use std::path::Path;

/// The fields of a pull request a watch reads.
const WATCH_FIELDS: &str = "state,isDraft,headRefOid,mergeStateStatus";
pub(super) struct GitHub;

impl Adapter for GitHub {
    fn state(&self, tools: &dyn Tools, url: &str) -> Result<PrState, ToolProblem> {
        state_of(&view(tools, url, "state,isDraft")?)
    }

    fn head(&self, tools: &dyn Tools, url: &str) -> Result<Head, ToolProblem> {
        let data = view(tools, url, "state,isDraft,headRefOid,headRefName")?;
        let text = |field: &str| data[field].as_str().unwrap_or("").to_string();
        Ok(Head {
            state: state_of(&data)?,
            commit: text("headRefOid"),
            branch: text("headRefName"),
        })
    }

    fn snapshot(&self, tools: &dyn Tools, url: &str) -> Result<Snapshot, ToolProblem> {
        let before = view(tools, url, WATCH_FIELDS)?;
        let state = state_of(&before)?;
        let (checks, after) = if matches!(state, PrState::Open | PrState::Draft) {
            // The pull request is read again after its checks, so the checks belong to the head it reports.
            (read_checks(tools, url)?, view(tools, url, WATCH_FIELDS)?)
        } else {
            (Vec::new(), before)
        };
        let base = match after["mergeStateStatus"].as_str().unwrap_or("") {
            "BEHIND" => Some(BaseState::Behind),
            "DIRTY" => Some(BaseState::Conflicts),
            // GitHub has not worked it out yet.
            "UNKNOWN" | "" => None,
            _ => Some(BaseState::UpToDate),
        };
        Ok(Snapshot {
            state: state_of(&after)?,
            head: after["headRefOid"].as_str().unwrap_or("").to_string(),
            applies: !checks.is_empty(),
            checks,
            base,
        })
    }

    /// The failed checks, each Actions run's log fetched once (the first failed check of a run carries it),
    /// for at most `MAX_LOGS` runs. A log that cannot be fetched leaves the name and link only.
    fn failures(&self, tools: &dyn Tools, checks: &[CheckResult]) -> Vec<Failure> {
        let mut fetched: Vec<&str> = Vec::new();
        let mut out = Vec::new();
        for check in checks.iter().filter(|check| check.bucket == "fail") {
            let mut text = None;
            if let Some((id, repository)) = run_of_link(&check.link)
                && !fetched.contains(&id)
                && fetched.len() < MAX_LOGS
            {
                fetched.push(id);
                text = tools
                    .gh(
                        None,
                        &["run", "view", id, "--repo", &repository, "--log-failed"],
                    )
                    .ok()
                    .filter(|output| output.code == Some(0))
                    .and_then(|output| excerpt(&output.stdout));
            }
            out.push(Failure {
                name: check.name.clone(),
                link: check.link.clone(),
                excerpt: text,
            });
        }
        out
    }

    fn find_open(
        &self,
        tools: &dyn Tools,
        dir: &Path,
        _origin: &Origin,
        branch: &str,
    ) -> Result<Option<OpenPr>, ToolProblem> {
        find_open(tools, dir, branch)
    }

    fn create(
        &self,
        tools: &dyn Tools,
        dir: &Path,
        _origin: &Origin,
        pr: &NewPr,
    ) -> Result<String, ToolProblem> {
        create(tools, dir, pr)
    }
}

/// The checks of the pull request at `url`, from `gh pr checks`.
fn read_checks(tools: &dyn Tools, url: &str) -> Result<Vec<CheckResult>, ToolProblem> {
    let pull_request = pull_request(url).ok_or_else(unsupported)?;
    let args = [
        "pr",
        "checks",
        pull_request.as_str(),
        "--json",
        "name,bucket,state,link",
    ];
    let output = run_gh(tools, None, &args)?;
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

fn state_of(data: &serde_json::Value) -> Result<PrState, ToolProblem> {
    match (data["state"].as_str(), data["isDraft"].as_bool()) {
        (Some("OPEN"), Some(true)) => Ok(PrState::Draft),
        (Some("OPEN"), _) => Ok(PrState::Open),
        (Some("MERGED"), _) => Ok(PrState::Merged),
        (Some("CLOSED"), _) => Ok(PrState::Closed),
        (other, _) => Err(ToolProblem(format!(
            "GitHub reported an unknown pull request state: {}",
            other.unwrap_or("none")
        ))),
    }
}

/// `gh pr view <pull request> --json <fields>`, parsed.
fn view(tools: &dyn Tools, url: &str, fields: &str) -> Result<serde_json::Value, ToolProblem> {
    let pull_request = pull_request(url).ok_or_else(unsupported)?;
    let output = run_gh(
        tools,
        None,
        &["pr", "view", &pull_request, "--json", fields],
    )?;
    if output.code != Some(0) {
        return Err(problem("pr view", &output));
    }
    serde_json::from_str(&output.stdout).map_err(|error| {
        ToolProblem(format!(
            "`gh pr view` gave output that is not JSON: {error}"
        ))
    })
}

/// The first open pull request of `branch` in the repository of `dir`.
///
/// `gh pr list --head` matches by branch name only, so a pull request from a fork that happens to use the
/// same name is skipped.
fn find_open(tools: &dyn Tools, dir: &Path, branch: &str) -> Result<Option<OpenPr>, ToolProblem> {
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
    let output = run_gh(tools, Some(dir), &args)?;
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
    Ok(Some(OpenPr {
        url: url.to_string(),
        base: item["baseRefName"]
            .as_str()
            .filter(|base| !base.is_empty())
            .map(str::to_string),
    }))
}

/// Opens `pr` with `gh pr create` in the repository of `dir` and returns its URL.
fn create(tools: &dyn Tools, dir: &Path, pr: &NewPr) -> Result<String, ToolProblem> {
    let body = pr.body_file.display().to_string();
    let output = run_gh(
        tools,
        Some(dir),
        &[
            "pr",
            "create",
            "--base",
            pr.base,
            "--head",
            pr.branch,
            "--title",
            pr.title,
            "--body-file",
            &body,
        ],
    )?;
    if output.code != Some(0) {
        return Err(problem("pr create", &output));
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
        })?;
    Ok(url.to_string())
}

/// A github.com repository identified by its origin URL.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GitHubRepo {
    pub(crate) owner: String,
    pub(crate) repository: String,
}

impl GitHubRepo {
    /// The github.com repository an origin URL (HTTPS or SSH, with or without `.git`) names; `None` for a URL
    /// of another host or one that does not name one repository.
    pub(crate) fn from_remote(remote: &str) -> Option<Self> {
        let path = remote_path(remote)?;
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        match path.split('/').collect::<Vec<_>>().as_slice() {
            [owner, repository] if !owner.is_empty() && !repository.is_empty() => Some(Self {
                owner: (*owner).to_string(),
                repository: (*repository).to_string(),
            }),
            _ => None,
        }
    }
}

/// Whether `remote` is a github.com origin URL in any of its usual spellings.
pub(super) fn is_remote(remote: &str) -> bool {
    remote_path(remote).is_some()
}

/// The repository path of a github.com remote, after the host; `None` for a remote of another host. The user
/// and port in the URL are ignored.
fn remote_path(remote: &str) -> Option<&str> {
    if let Some(path) = remote.strip_prefix("git@github.com:") {
        return Some(path);
    }
    let (_, rest) = remote.split_once("://")?;
    let (authority, path) = rest.split_once('/')?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    host.eq_ignore_ascii_case("github.com").then_some(path)
}

/// The GitHub (or GitHub Enterprise) pull request a link points to, as `<repository>/pull/<number>`: a link to
/// one of its tabs (`/files`) or comments (`#issuecomment-1`) is cut back to the pull request. None for a link
/// that is not a pull request.
pub(super) fn pull_request(url: &str) -> Option<String> {
    let page = url.split(['?', '#']).next().unwrap_or(url);
    // The last /pull/<number>, so a repository that is itself called pull still works.
    page.rmatch_indices("/pull/").find_map(|(at, marker)| {
        let number = page[at + marker.len()..].split('/').next().unwrap_or("");
        (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("{}/pull/{number}", &page[..at]))
    })
}

/// Runs `gh` with `args`, in `dir` when given.
///
/// # Errors
///
/// Returns a problem that says what to do when `gh` cannot be started.
fn run_gh(tools: &dyn Tools, dir: Option<&Path>, args: &[&str]) -> Result<ToolOutput, ToolProblem> {
    started(tools.gh(dir, args))
}

/// What `gh` printed, or a problem that says what to do when it could not be started.
fn started(result: Result<ToolOutput, ToolError>) -> Result<ToolOutput, ToolProblem> {
    result.map_err(|error| match error {
        ToolError::NotInstalled => ToolProblem(
            "The GitHub CLI (`gh`) is not installed or not on PATH. Install it from https://cli.github.com, run `gh auth login`, and try again."
                .into(),
        ),
        ToolError::Other(text) => ToolProblem(format!("Could not run `gh`: {text}")),
    })
}

/// A failed `gh <what>` call as a message that says what to do.
fn problem(what: &str, output: &ToolOutput) -> ToolProblem {
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
