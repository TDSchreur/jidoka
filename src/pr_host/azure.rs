//! The Azure Repos adapter: reads pull requests on Azure DevOps Services with `az`, and identifies Azure
//! repositories from their URLs.
//!
//! A pull request has two heads here. `head` reads `lastMergeSourceCommit`, which Azure keeps when it deletes
//! the source branch on completion, so Run cleanup can still match a merged pull request to its branch.
//! Watching CI reads the live source branch ref instead (`AzurePr::live_head`), which shows a push at once.

use super::{
    Adapter, Head, Host, NewPr, OpenPr, Origin, ToolError, ToolOutput, ToolProblem, Tools,
    encode_segment, watch::Snapshot,
};
use crate::{
    pr_state::PrState,
    pr_wait::{BaseState, CheckResult},
};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub(super) struct Azure;

impl Adapter for Azure {
    fn state(&self, tools: &dyn Tools, url: &str) -> Result<PrState, ToolProblem> {
        state_of(&AzurePr::parse(url)?.fetch(tools)?)
    }

    fn head(&self, tools: &dyn Tools, url: &str) -> Result<Head, ToolProblem> {
        let data = AzurePr::parse(url)?.fetch(tools)?;
        let commit = required(&data["lastMergeSourceCommit"]["commitId"], "source commit")?;
        let branch = branch_name(required(&data["sourceRefName"], "source branch")?)?;
        Ok(Head {
            state: state_of(&data)?,
            commit: commit.to_string(),
            branch: branch.to_string(),
        })
    }

    /// The pull request's state and live head are read before and after its checks; a pull request that is
    /// no longer active has no checks and keeps its last source commit as its head.
    fn snapshot(&self, tools: &dyn Tools, url: &str) -> Result<Snapshot, ToolProblem> {
        let pr = AzurePr::parse(url)?;
        let before = pr.show(tools)?;
        if !is_active(&before)? {
            return ended(&before);
        }
        let head = pr.live_head(tools, &before)?;
        let (checks, applies) = checks_for(&pr, tools, &before, &head, url)?;
        let after = pr.show(tools)?;
        if !is_active(&after)? {
            return ended(&after);
        }
        Ok(Snapshot {
            state: state_of(&after)?,
            head: pr.live_head(tools, &after)?,
            checks,
            applies,
            base: Some(base_state(&after)),
        })
    }

    fn find_open(
        &self,
        tools: &dyn Tools,
        _dir: &Path,
        origin: &Origin,
        branch: &str,
    ) -> Result<Option<OpenPr>, ToolProblem> {
        find_open(tools, repo_of(origin)?, branch)
    }

    fn check_new(&self, pr: &NewPr) -> Result<(), ToolProblem> {
        description(pr).map(drop)
    }

    fn create(
        &self,
        tools: &dyn Tools,
        _dir: &Path,
        origin: &Origin,
        pr: &NewPr,
    ) -> Result<String, ToolProblem> {
        create(tools, repo_of(origin)?, pr)
    }
}

/// Whether the pull request is still active; a pull request in a state Jidoka does not know is an error.
fn is_active(data: &Value) -> Result<bool, ToolProblem> {
    Ok(matches!(state_of(data)?, PrState::Open | PrState::Draft))
}

/// The snapshot of a pull request that is no longer active.
fn ended(data: &Value) -> Result<Snapshot, ToolProblem> {
    Ok(Snapshot {
        state: state_of(data)?,
        head: data["lastMergeSourceCommit"]["commitId"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        checks: Vec::new(),
        applies: false,
        base: Some(BaseState::Unknown),
    })
}

/// Azure's `active`, `completed` and `abandoned`, with `isDraft` for an active draft.
fn state_of(data: &Value) -> Result<PrState, ToolProblem> {
    let draft = data["isDraft"].as_bool().unwrap_or(false);
    match required(&data["status"], "pull request status")? {
        "active" if draft => Ok(PrState::Draft),
        "active" => Ok(PrState::Open),
        "completed" => Ok(PrState::Merged),
        "abandoned" => Ok(PrState::Closed),
        status => Err(ToolProblem(format!(
            "Azure reported an unknown pull request status: {status}"
        ))),
    }
}

/// The Azure repository of `origin`. `Hosts` picks this adapter for an Azure origin only.
fn repo_of(origin: &Origin) -> Result<&AzureRepo, ToolProblem> {
    match &origin.host {
        Host::Azure(repo) => Ok(repo),
        _ => Err(ToolProblem("The origin is not an Azure repository.".into())),
    }
}

/// Azure DevOps rejects a pull request description longer than this many characters.
const DESCRIPTION_LIMIT: usize = 4000;

/// The first active pull request from `branch` in `repo`, skipping pull requests from forks.
fn find_open(
    tools: &dyn Tools,
    repo: &AzureRepo,
    branch: &str,
) -> Result<Option<OpenPr>, ToolProblem> {
    let org = repo.org_url();
    let data = az_json(
        tools,
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
    // A pull request from a fork with the same branch name must not be reused.
    let Some(item) = list.iter().find(|item| item["forkSource"].is_null()) else {
        return Ok(None);
    };
    let id = item["pullRequestId"].as_u64().ok_or_else(|| {
        ToolProblem("Azure PR list returned a pull request without an ID.".into())
    })?;
    let base = item["targetRefName"]
        .as_str()
        .and_then(|s| s.strip_prefix("refs/heads/"))
        .map(str::to_string);
    Ok(Some(OpenPr {
        url: repo.pr_url(id),
        base,
    }))
}

/// The `@<file>` argument that hands `az` the description of `pr`, after checking that Azure takes it.
///
/// # Errors
///
/// Returns a problem when the body is not UTF-8, too long or has NUL bytes, or when `az` on Windows
/// cannot take the title or the file's path.
fn description(pr: &NewPr) -> Result<String, ToolProblem> {
    let text = std::fs::read_to_string(pr.body_file)
        .map_err(|_| ToolProblem("The pull request body could not be read as UTF-8.".into()))?;
    let length = text.chars().count();
    if length > DESCRIPTION_LIMIT {
        return Err(ToolProblem(format!(
            "The pull request body has {length} characters; Azure allows at most {DESCRIPTION_LIMIT}. Cut at least {}.",
            length - DESCRIPTION_LIMIT
        )));
    }
    if text.contains('\0') {
        return Err(ToolProblem(
            "The pull request body must not contain NUL bytes.".into(),
        ));
    }
    let file = file_argument(&pr.body_file.to_string_lossy());
    check_text("title", pr.title, cfg!(windows))?;
    check_text("path of the body file", &file, cfg!(windows))?;
    // `az` reads `@<file>` itself. A multi-line body cannot be an argument of `az.cmd` on Windows.
    Ok(file)
}

/// Windows runs `az.cmd` through `cmd.exe`, and Rust refuses arguments that `cmd.exe` would re-parse.
/// Checking them before the push keeps a refusal from leaving a pushed branch without a pull request.
fn check_text(what: &str, text: &str, windows: bool) -> Result<(), ToolProblem> {
    if windows
        && text
            .chars()
            .any(|c| matches!(c, '"' | '%') || c.is_control())
    {
        return Err(ToolProblem(format!(
            "The {what} must not contain double quotes, percent signs or control characters; Azure CLI on Windows cannot take them."
        )));
    }
    Ok(())
}

/// The `@<file>` value that makes `az` read an argument from a file. Canonical Windows paths carry a
/// `\\?\` prefix that is dropped, so `\\?\UNC\server\share` becomes `\\server\share`.
fn file_argument(path: &str) -> String {
    let plain = match path.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(path).to_string(),
    };
    format!("@{plain}")
}

/// Opens `pr` in `repo` with `az repos pr create` and returns its canonical URL.
fn create(tools: &dyn Tools, repo: &AzureRepo, pr: &NewPr) -> Result<String, ToolProblem> {
    let description = description(pr)?;
    let org = repo.org_url();
    let title = format!("--title={}", pr.title);
    let data = az_json(
        tools,
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
            pr.branch,
            "--target-branch",
            pr.base,
            &title,
            "--description",
            &description,
            "--output",
            "json",
            "--detect",
            "false",
        ],
    )?;
    let id = data["pullRequestId"].as_u64().ok_or_else(|| {
        ToolProblem("Azure PR creation returned no pull request ID; check for an existing PR before retrying.".into())
    })?;
    Ok(repo.pr_url(id))
}

/// Policies asked for per `az repos pr policy list` call.
const POLICY_PAGE_SIZE: usize = 100;
/// Pages of policies read before giving up, so a misbehaving response cannot loop forever.
const MAX_POLICY_PAGES: usize = 100;
/// Configuration type ID of the Azure Repos Build policy.
const BUILD_POLICY_TYPE: &str = "0609b952-1397-4640-95ec-e00a01b2c241";

fn parse_time(value: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

/// Azure's merge calculation proves conflicts; a successful merge does not prove the branch is up to date.
fn base_state(data: &Value) -> BaseState {
    match data["mergeStatus"].as_str() {
        Some("conflicts") => BaseState::Conflicts,
        _ => BaseState::Unknown,
    }
}

/// The checks of `head` from the policies and statuses of the latest iteration, and whether any of them
/// applies. While Azure has not caught up with `head`, one pending check says so.
#[expect(
    clippy::too_many_lines,
    reason = "walks iterations, statuses and policy evaluations into check results in one go"
)]
fn checks_for(
    pr: &AzurePr,
    tools: &dyn Tools,
    data: &Value,
    head: &str,
    url: &str,
) -> Result<(Vec<CheckResult>, bool), ToolProblem> {
    let repo = required(&data["repository"]["id"], "repository ID")?;
    let iterations = pr.invoke(tools, "pullRequestIterations", repo, None)?;
    let latest = iterations
        .iter()
        .max_by_key(|item| item["id"].as_u64().unwrap_or(0));
    let mut checks = Vec::new();
    let current = latest
        .is_some_and(|item| item["sourceRefCommit"]["commitId"].as_str() == Some(head))
        && data["lastMergeSourceCommit"]["commitId"].as_str() == Some(head);
    if !current {
        checks.push(CheckResult {
            name: "Azure is updating the current PR iteration".into(),
            bucket: "pending".into(),
            link: url.into(),
        });
        return Ok((checks, true));
    }
    let iteration_time = latest
        .and_then(|item| item["createdDate"].as_str())
        .and_then(parse_time);
    let iteration = latest
        .and_then(|item| item["id"].as_u64())
        .filter(|id| *id > 0)
        .ok_or_else(|| ToolProblem("Azure PR iteration has no valid ID.".into()))?;
    let mut policies = Vec::new();
    let top = POLICY_PAGE_SIZE.to_string();
    for page in 0..MAX_POLICY_PAGES {
        let skip = (page * POLICY_PAGE_SIZE).to_string();
        let response = az_json(
            tools,
            &[
                "repos",
                "pr",
                "policy",
                "list",
                "--id",
                &pr.id,
                "--organization",
                &pr.org,
                "--top",
                &top,
                "--skip",
                &skip,
                "--output",
                "json",
                "--detect",
                "false",
            ],
        )?;
        let items = response
            .as_array()
            .ok_or_else(|| ToolProblem("Azure policy response is not an array.".into()))?;
        policies.extend(items.iter().cloned());
        if items.len() < POLICY_PAGE_SIZE {
            break;
        }
        if page + 1 == MAX_POLICY_PAGES {
            return Err(ToolProblem(
                "Azure returned too many policies to evaluate safely.".into(),
            ));
        }
    }
    let mut applicable = false;
    for policy in &policies {
        let enabled = policy["configuration"]["isEnabled"]
            .as_bool()
            .ok_or_else(|| {
                ToolProblem("Azure policy does not say whether it is enabled.".into())
            })?;
        if !enabled {
            continue;
        }
        let required_policy = policy["configuration"]["isBlocking"]
            .as_bool()
            .ok_or_else(|| {
                ToolProblem("Azure policy does not say whether it is required.".into())
            })?;
        let state = required(&policy["status"], "policy state")?;
        if required_policy && state != "notApplicable" {
            applicable = true;
        }
        let build_id = policy["context"]["buildId"].as_u64();
        let build_policy = build_id.is_some()
            || policy["configuration"]["type"]["id"].as_str() == Some(BUILD_POLICY_TYPE)
            || policy["configuration"]["type"]["displayName"].as_str() == Some("Build");
        // Azure re-evaluates other policies itself; only a build must have started for the current iteration.
        let evaluation_time = policy["startedDate"].as_str().and_then(parse_time);
        let fresh = !build_policy
            || matches!((iteration_time, evaluation_time), (Some(iteration), Some(evaluation)) if evaluation >= iteration);
        let bucket = if !required_policy || state == "notApplicable" {
            "skipping"
        } else if !fresh
            || policy["context"]["isExpired"].as_bool() == Some(true)
            || policy["context"]["buildIsNotCurrent"].as_bool() == Some(true)
        {
            "pending"
        } else {
            match (state, build_id) {
                ("approved", _) => "pass",
                ("rejected", Some(id)) => {
                    let id = id.to_string();
                    // The lookup only tells a cancelled build from a failed one; it never undoes the rejection.
                    let build = az_json(
                        tools,
                        &[
                            "pipelines",
                            "runs",
                            "show",
                            "--id",
                            &id,
                            "--organization",
                            &pr.org,
                            "--project",
                            &pr.project,
                            "--output",
                            "json",
                            "--detect",
                            "false",
                        ],
                    );
                    match build {
                        Ok(build)
                            if build["status"].as_str() == Some("completed")
                                && matches!(
                                    build["result"].as_str(),
                                    Some("canceled" | "cancelled")
                                ) =>
                        {
                            "cancel"
                        }
                        _ => "fail",
                    }
                }
                ("rejected" | "broken", _) => "fail",
                _ => "pending",
            }
        };
        let name = policy["configuration"]["type"]["displayName"]
            .as_str()
            .unwrap_or("Azure policy");
        let link = build_id
            .map(|id| {
                format!(
                    "{}/{}/_build/results?buildId={id}",
                    pr.org,
                    encode_segment(&pr.project)
                )
            })
            .unwrap_or_else(|| url.into());
        checks.push(CheckResult {
            name: if required_policy {
                name.into()
            } else {
                format!("{name} (optional: {state})")
            },
            bucket: bucket.into(),
            link,
        });
    }
    let mut statuses = pr.invoke(tools, "pullRequestStatuses", repo, None)?;
    statuses.extend(pr.invoke(tools, "pullRequestIterationStatuses", repo, Some(iteration))?);
    let mut latest_status: BTreeMap<(String, String), Value> = BTreeMap::new();
    for status in statuses {
        if status["iterationId"]
            .as_u64()
            .is_some_and(|id| id != iteration)
        {
            continue;
        }
        let name = required(&status["context"]["name"], "status name")?.to_string();
        let genre = status["context"]["genre"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let id = status["id"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or_else(|| ToolProblem("Azure status has no valid ID.".into()))?;
        let key = (genre, name);
        if latest_status
            .get(&key)
            .and_then(|item| item["id"].as_u64())
            .unwrap_or(0)
            < id
        {
            latest_status.insert(key, status);
        }
    }
    for ((genre, name), status) in latest_status {
        let state = required(&status["state"], "status state")?;
        let scoped = status["iterationId"].as_u64() == Some(iteration);
        if !scoped || state != "notApplicable" {
            applicable = true;
        }
        let bucket = if !scoped {
            "pending"
        } else {
            match state {
                "failed" | "error" => "fail",
                "canceled" | "cancelled" => "cancel",
                "notApplicable" => "skipping",
                "succeeded" => "pass",
                _ => "pending",
            }
        };
        checks.push(CheckResult {
            name: if genre.is_empty() {
                name
            } else {
                format!("{genre}: {name}")
            },
            bucket: bucket.into(),
            link: url.into(),
        });
    }
    Ok((checks, applicable))
}

/// The branch name of a source ref such as `refs/heads/feature/x`.
fn branch_name(reference: &str) -> Result<&str, ToolProblem> {
    reference
        .strip_prefix("refs/heads/")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolProblem("Azure pull request has an invalid source branch.".into()))
}

/// An Azure Repos pull request, as its URL names it.
struct AzurePr {
    /// The organization URL, `https://dev.azure.com/<organization>`.
    org: String,
    project: String,
    repository: String,
    id: String,
}

impl AzurePr {
    /// Reads the canonical `https://dev.azure.com/<organization>/<project>/_git/<repository>/pullrequest/<id>` URL.
    fn parse(url: &str) -> Result<Self, ToolProblem> {
        let Some(path) = url.strip_prefix("https://dev.azure.com/") else {
            return Err(ToolProblem(
                "Legacy visualstudio.com pull request URLs are not supported; record the canonical https://dev.azure.com/<organization>/<project>/_git/<repository>/pullrequest/<id> URL with `jidoka run update <run-id> --pr <url>`.".into(),
            ));
        };
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() != 6
            || parts[2] != "_git"
            || parts[4] != "pullrequest"
            || parts[5].is_empty()
            || parts[5].parse::<u64>().ok().filter(|id| *id > 0).is_none()
            || !parts[5].bytes().all(|b| b.is_ascii_digit())
        {
            return Err(ToolProblem("Invalid Azure pull request URL.".into()));
        }
        let decode = |part: &str| {
            decode_segment(part)
                .ok_or_else(|| ToolProblem("Invalid Azure pull request URL.".into()))
        };
        let org = decode(parts[0])?;
        let project = decode(parts[1])?;
        let repository = decode(parts[3])?;
        Ok(Self {
            org: format!("https://dev.azure.com/{}", encode_segment(&org)),
            project,
            repository,
            id: parts[5].into(),
        })
    }
    /// The pull request as Azure reports it, checked to be the one the URL names.
    fn fetch(&self, tools: &dyn Tools) -> Result<Value, ToolProblem> {
        let data = az_json(
            tools,
            &[
                "repos",
                "pr",
                "show",
                "--id",
                &self.id,
                "--organization",
                &self.org,
                "--output",
                "json",
                "--detect",
                "false",
            ],
        )?;
        if data["repository"]["name"].as_str() != Some(self.repository.as_str())
            || data["repository"]["project"]["name"].as_str() != Some(self.project.as_str())
            || data["pullRequestId"].as_u64() != self.id.parse::<u64>().ok()
        {
            return Err(ToolProblem(
                "Azure pull request does not match the recorded repository.".into(),
            ));
        }
        Ok(data)
    }
    /// The pull request, refused when it comes from a fork.
    fn show(&self, tools: &dyn Tools) -> Result<Value, ToolProblem> {
        let data = self.fetch(tools)?;
        if !data["forkSource"].is_null() {
            return Err(ToolProblem(
                "Watching pull requests from Azure forks is not supported yet.".into(),
            ));
        }
        Ok(data)
    }
    /// The commit the source branch has now, read from the live branch ref.
    fn live_head(&self, tools: &dyn Tools, data: &Value) -> Result<String, ToolProblem> {
        let reference = required(&data["sourceRefName"], "source branch")?;
        let branch = branch_name(reference)?;
        let repo_id = required(&data["repository"]["id"], "repository ID")?;
        let filter = format!("--filter=heads/{branch}");
        let items = az_json(
            tools,
            &[
                "repos",
                "ref",
                "list",
                "--organization",
                &self.org,
                "--project",
                &self.project,
                "--repository",
                repo_id,
                &filter,
                "--output",
                "json",
                "--detect",
                "false",
            ],
        )?;
        let list = items
            .as_array()
            .ok_or_else(|| ToolProblem("Azure refs response is not an array.".into()))?;
        let found = list
            .iter()
            .find(|item| item["name"].as_str() == Some(reference))
            .ok_or_else(|| ToolProblem("Azure source branch was not found.".into()))?;
        Ok(required(&found["objectId"], "source head")?.to_string())
    }
    /// The `value` list of a pull request resource read with `az devops invoke`.
    fn invoke(
        &self,
        tools: &dyn Tools,
        resource: &str,
        repository_id: &str,
        iteration: Option<u64>,
    ) -> Result<Vec<Value>, ToolProblem> {
        let project = format!("project={}", self.project);
        let repo = format!("repositoryId={repository_id}");
        let id = format!("pullRequestId={}", self.id);
        let iteration_arg = iteration.map(|value| format!("iterationId={value}"));
        let mut args = vec![
            "devops",
            "invoke",
            "--organization",
            &self.org,
            "--area",
            "git",
            "--resource",
            resource,
            "--route-parameters",
            &project,
            &repo,
            &id,
        ];
        if let Some(value) = &iteration_arg {
            args.push(value);
        }
        args.extend([
            "--api-version",
            "7.1",
            "--output",
            "json",
            "--detect",
            "false",
        ]);
        let data = az_json(tools, &args)?;
        if !data["continuation_token"].is_null() {
            return Err(ToolProblem(
                "Azure returned a paginated response; cannot safely evaluate all checks.".into(),
            ));
        }
        data["value"]
            .as_array()
            .cloned()
            .ok_or_else(|| ToolProblem("Azure returned an invalid checks response.".into()))
    }
}

/// Azure DevOps Services repository identified by its origin URL.
#[derive(PartialEq, Eq)]
pub(crate) struct AzureRepo {
    pub(crate) organization: String,
    pub(crate) project: String,
    pub(crate) repository: String,
}

impl AzureRepo {
    /// The Azure repository an origin URL (HTTPS or SSH, canonical or legacy) names; `None` for a URL of
    /// another host.
    ///
    /// # Errors
    ///
    /// Returns a problem for an Azure URL that does not name one repository.
    pub(crate) fn from_remote(remote: &str) -> Result<Option<Self>, ToolProblem> {
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
        let decode = |part: &str| {
            decode_segment(part)
                .ok_or_else(|| ToolProblem("Invalid Azure Repos origin URL.".into()))
        };
        Ok(Some(Self {
            organization: decode(parts[0])?,
            project: decode(parts[1])?,
            repository: decode(parts[2])?,
        }))
    }
    /// The organization URL, `https://dev.azure.com/<organization>`.
    pub(crate) fn org_url(&self) -> String {
        format!(
            "https://dev.azure.com/{}",
            encode_segment(&self.organization)
        )
    }
    /// The canonical URL of pull request `id` in this repository.
    pub(crate) fn pr_url(&self, id: u64) -> String {
        format!(
            "{}/{}/_git/{}/pullrequest/{id}",
            self.org_url(),
            encode_segment(&self.project),
            encode_segment(&self.repository)
        )
    }
}

/// One percent-encoded segment of an Azure URL, decoded. `None` when it is not a plain name, so a decoded
/// value can never become a path, a query or an option.
fn decode_segment(part: &str) -> Option<String> {
    let value = percent_encoding::percent_decode_str(part)
        .decode_utf8()
        .ok()?
        .into_owned();
    let plain = !value.is_empty()
        && !value.starts_with('-')
        && value != "."
        && value != ".."
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '?' | '#' | '@' | '%'));
    plain.then_some(value)
}

/// A string field of an Azure response that is safe to pass on: not empty, no option, no control characters.
///
/// # Errors
///
/// Returns a problem naming `label` when the field is missing or not safe.
fn required<'a>(value: &'a Value, label: &str) -> Result<&'a str, ToolProblem> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && !s.starts_with('-') && !s.chars().any(char::is_control))
        .ok_or_else(|| ToolProblem(format!("Azure response has no valid {label}.")))
}

/// Runs `az` with `args` and parses the JSON it prints.
///
/// # Errors
///
/// Returns a problem that says what to do when `az` cannot be started, fails or prints no JSON.
fn az_json(tools: &dyn Tools, args: &[&str]) -> Result<Value, ToolProblem> {
    az_parsed(args, tools.az(args))
}

/// The JSON that `az <args>` printed, as `result` holds it. Azure's own output is never echoed: it can
/// contain credentials or other sensitive data.
fn az_parsed(args: &[&str], result: Result<ToolOutput, ToolError>) -> Result<Value, ToolProblem> {
    let operation = match args {
        ["repos", "pr", "show", ..] => "read the pull request",
        ["repos", "pr", "list", ..] => "list pull requests",
        ["repos", "pr", "create", ..] => "create the pull request",
        ["repos", "pr", "policy", "list", ..] => "list pull request policies",
        ["repos", "ref", "list", ..] => "read the source branch",
        ["pipelines", "runs", "show", ..] => "read the policy build",
        ["devops", "invoke", ..] => "read pull request checks",
        _ => "read Azure DevOps",
    };
    let result = result.map_err(|error| match error {
        ToolError::NotInstalled => ToolProblem(
            "Azure CLI (`az`) is not installed; install it and the azure-devops extension.".into(),
        ),
        ToolError::Other(_) => ToolProblem(format!(
            "Could not start Azure CLI to {operation}; check executable permissions."
        )),
    })?;
    if result.code != Some(0) {
        let error = result.stderr.to_lowercase();
        let advice = if error.contains("az login")
            || error.contains("az devops login")
            || error.contains("authentication")
            || error.contains("unauthorized")
            || error.contains("401")
        {
            "check Azure authentication with az login or az devops login"
        } else if error.contains("extension")
            || error.contains("not recognized")
            || error.contains("unrecognized arguments")
        {
            "install or update the azure-devops extension"
        } else if error.contains("forbidden")
            || error.contains("permission")
            || error.contains("403")
        {
            "check your repository and pipeline permissions"
        } else if error.contains("connection")
            || error.contains("timed out")
            || error.contains("network")
        {
            "check the network connection"
        } else {
            "check the azure-devops extension, authentication, and repository access"
        };
        return Err(ToolProblem(format!(
            "Azure CLI could not {operation}; {advice}."
        )));
    }
    serde_json::from_str(&result.stdout).map_err(|error| {
        ToolProblem(format!(
            "Azure CLI returned invalid JSON while trying to {operation} (line {}, column {}).",
            error.line(),
            error.column()
        ))
    })
}
