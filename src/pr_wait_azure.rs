//! Read-only Azure DevOps Services pull request status watching.
use super::{
    BaseState, CheckResult, Clock, Failure, GRACE, Gh, GhError, Options, Outcome, POLL,
    ToolProblem, Waited,
};
use serde_json::Value;
use std::collections::BTreeMap;

/// Policies asked for per `az repos pr policy list` call.
const POLICY_PAGE_SIZE: usize = 100;
/// Pages of policies read before giving up, so a misbehaving response cannot loop forever.
const MAX_POLICY_PAGES: usize = 100;
/// Configuration type ID of the Azure Repos Build policy.
const BUILD_POLICY_TYPE: &str = "0609b952-1397-4640-95ec-e00a01b2c241";

struct AzurePr {
    org: String,
    project: String,
    repository: String,
    id: String,
}

impl AzurePr {
    fn parse(url: &str) -> Result<Self, ToolProblem> {
        let Some(path) = url.strip_prefix("https://dev.azure.com/") else {
            return Err(ToolProblem(
                "Legacy visualstudio.com pull request URLs are not supported; record the canonical https://dev.azure.com/<organization>/<project>/_git/<repository>/pullrequest/<id> URL with `jidoka run update <run-id> running --pr <url>`.".into(),
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
        let decode = |s: &str| -> Result<String, ToolProblem> {
            let value = percent_encoding::percent_decode_str(s)
                .decode_utf8()
                .map_err(|_| ToolProblem("Invalid Azure pull request URL.".into()))?
                .into_owned();
            if value.is_empty()
                || value.starts_with('-')
                || value == "."
                || value == ".."
                || value
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\' | '?' | '#' | '@' | '%'))
            {
                return Err(ToolProblem("Invalid Azure pull request URL.".into()));
            }
            Ok(value)
        };
        let org = decode(parts[0])?;
        let project = decode(parts[1])?;
        let repository = decode(parts[3])?;
        Ok(Self {
            org: format!(
                "https://dev.azure.com/{}",
                crate::pr_open::encode_segment(&org)
            ),
            project,
            repository,
            id: parts[5].into(),
        })
    }
    /// The pull request as Azure reports it, checked to be the one the URL names.
    fn fetch(&self, gh: &dyn Gh) -> Result<Value, ToolProblem> {
        let data = az_json(
            gh,
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
    fn show(&self, gh: &dyn Gh) -> Result<Value, ToolProblem> {
        let data = self.fetch(gh)?;
        if !data["forkSource"].is_null() {
            return Err(ToolProblem(
                "Watching pull requests from Azure forks is not supported yet.".into(),
            ));
        }
        Ok(data)
    }
    fn head(&self, gh: &dyn Gh, data: &Value) -> Result<String, ToolProblem> {
        let reference = required(&data["sourceRefName"], "source branch")?;
        let branch = reference
            .strip_prefix("refs/heads/")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ToolProblem("Azure pull request has an invalid source branch.".into())
            })?;
        let repo_id = required(&data["repository"]["id"], "repository ID")?;
        let filter = format!("--filter=heads/{branch}");
        let items = az_json(
            gh,
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
    fn invoke(
        &self,
        gh: &dyn Gh,
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
        let data = az_json(gh, &args)?;
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

/// The status (`active`, `completed` or `abandoned`) and draft flag of the Azure Repos pull request at `url`.
pub(super) fn status(gh: &dyn Gh, url: &str) -> Result<(String, bool), ToolProblem> {
    let data = AzurePr::parse(url)?.fetch(gh)?;
    let status = required(&data["status"], "pull request status")?.to_string();
    Ok((status, data["isDraft"].as_bool().unwrap_or(false)))
}

/// The last source commit and the source branch name of the Azure Repos pull request at `url`.
pub(super) fn head(gh: &dyn Gh, url: &str) -> Result<(String, String), ToolProblem> {
    let data = AzurePr::parse(url)?.fetch(gh)?;
    let commit = required(&data["lastMergeSourceCommit"]["commitId"], "source commit")?;
    let reference = required(&data["sourceRefName"], "source branch")?;
    let branch = reference
        .strip_prefix("refs/heads/")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolProblem("Azure pull request has an invalid source branch.".into()))?;
    Ok((commit.to_string(), branch.to_string()))
}

fn required<'a>(value: &'a Value, label: &str) -> Result<&'a str, ToolProblem> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && !s.starts_with('-') && !s.chars().any(char::is_control))
        .ok_or_else(|| ToolProblem(format!("Azure response has no valid {label}.")))
}

fn parse_time(value: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

fn az_json(gh: &dyn Gh, args: &[&str]) -> Result<Value, ToolProblem> {
    let operation = match args {
        ["repos", "pr", "show", ..] => "read the pull request",
        ["repos", "pr", "policy", "list", ..] => "list pull request policies",
        ["repos", "ref", "list", ..] => "read the source branch",
        ["pipelines", "runs", "show", ..] => "read the policy build",
        ["devops", "invoke", ..] => "read pull request checks",
        _ => "read Azure DevOps",
    };
    let result = gh.az(args).map_err(|error| match error {
        GhError::NotInstalled => ToolProblem(
            "Azure CLI (`az`) is not installed; install it and the azure-devops extension.".into(),
        ),
        GhError::Other(_) => ToolProblem(format!(
            "Could not start Azure CLI to {operation}; check executable permissions."
        )),
    })?;
    if result.code != Some(0) {
        // Do not echo Azure output: it can contain credentials or other sensitive data.
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

/// Azure's merge calculation proves conflicts; a successful merge does not prove the branch is up to date.
fn base_state(data: &Value) -> BaseState {
    match data["mergeStatus"].as_str() {
        Some("conflicts") => BaseState::Conflicts,
        _ => BaseState::Unknown,
    }
}

fn result(
    url: &str,
    head: &str,
    current: &str,
    outcome: Outcome,
    checks: Vec<CheckResult>,
    base: BaseState,
) -> Waited {
    let failures = checks
        .iter()
        .filter(|check| check.bucket == "fail")
        .map(|check| Failure {
            name: check.name.clone(),
            link: check.link.clone(),
            excerpt: None,
        })
        .collect();
    Waited {
        outcome,
        exit_code: outcome.exit_code(),
        pull_request: url.into(),
        head: head.into(),
        new_head: (head != current).then(|| current.into()),
        base,
        checks,
        failures,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "walks iterations, statuses and policy evaluations into check results in one go"
)]
fn checks_for(
    pr: &AzurePr,
    gh: &dyn Gh,
    data: &Value,
    head: &str,
    url: &str,
) -> Result<(Vec<CheckResult>, bool), ToolProblem> {
    let repo = required(&data["repository"]["id"], "repository ID")?;
    let iterations = pr.invoke(gh, "pullRequestIterations", repo, None)?;
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
            gh,
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
                        gh,
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
                    crate::pr_open::encode_segment(&pr.project)
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
    let mut statuses = pr.invoke(gh, "pullRequestStatuses", repo, None)?;
    statuses.extend(pr.invoke(gh, "pullRequestIterationStatuses", repo, Some(iteration))?);
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

/// Whether the pull request is still active; `completed` and `abandoned` are not, anything else is an error.
fn is_active(data: &Value) -> Result<bool, ToolProblem> {
    match required(&data["status"], "pull request status")? {
        "active" => Ok(true),
        "completed" | "abandoned" => Ok(false),
        _ => Err(ToolProblem(
            "Azure returned an unknown pull request status.".into(),
        )),
    }
}

/// Ends the wait when the pull request is no longer open or its source moved away from `head`.
fn moved_on(
    pr: &AzurePr,
    gh: &dyn Gh,
    data: &Value,
    url: &str,
    head: &str,
    activity: &mut dyn FnMut(&str),
) -> Result<Option<Waited>, ToolProblem> {
    if !is_active(data)? {
        activity("Pull request is not open");
        return Ok(Some(result(
            url,
            head,
            head,
            Outcome::NotOpen,
            vec![],
            BaseState::Unknown,
        )));
    }
    let current = pr.head(gh, data)?;
    if current != head {
        activity("Head commit changed while waiting for CI");
        return Ok(Some(result(
            url,
            head,
            &current,
            Outcome::HeadChanged,
            vec![],
            BaseState::Unknown,
        )));
    }
    Ok(None)
}

pub(super) fn wait(
    url: &str,
    options: &Options,
    gh: &dyn Gh,
    clock: &dyn Clock,
    activity: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(&str),
) -> Result<Waited, ToolProblem> {
    let pr = AzurePr::parse(url)?;
    let first = pr.show(gh)?;
    if !is_active(&first)? {
        let head = first["lastMergeSourceCommit"]["commitId"]
            .as_str()
            .unwrap_or("");
        activity("Pull request is not open");
        return Ok(result(
            url,
            head,
            head,
            Outcome::NotOpen,
            vec![],
            BaseState::Unknown,
        ));
    }
    let head = pr.head(gh, &first)?;
    activity("Waiting for CI");
    let started = clock.elapsed();
    loop {
        let data = pr.show(gh)?;
        if let Some(waited) = moved_on(&pr, gh, &data, url, &head, activity)? {
            return Ok(waited);
        }
        let (checks, applicable) = checks_for(&pr, gh, &data, &head, url)?;
        let after = pr.show(gh)?;
        if let Some(waited) = moved_on(&pr, gh, &after, url, &head, activity)? {
            return Ok(waited);
        }
        let elapsed = clock.elapsed().saturating_sub(started);
        let pending = checks.iter().any(|check| check.bucket == "pending");
        let failed = checks.iter().any(|check| check.bucket == "fail");
        let outcome = if failed && (options.fail_fast || !pending) {
            Some(Outcome::Failed)
        } else if pending && elapsed >= options.timeout {
            Some(Outcome::Timeout)
        } else if pending {
            None
        } else if !applicable && elapsed >= GRACE.min(options.timeout) {
            Some(Outcome::NoCi)
        } else if !applicable {
            None
        } else if checks.iter().any(|check| check.bucket == "cancel") {
            Some(Outcome::Cancelled)
        } else {
            Some(Outcome::Passed)
        };
        if let Some(outcome) = outcome {
            let waited = result(url, &head, &head, outcome, checks, base_state(&after));
            activity(&super::activity_text(&waited));
            return Ok(waited);
        }
        progress(if pending {
            "Azure checks or policy evaluations are pending"
        } else {
            "No checks reported yet"
        });
        clock.sleep(POLL.min(options.timeout.saturating_sub(elapsed)));
    }
}
