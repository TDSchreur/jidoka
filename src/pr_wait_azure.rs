//! Read-only Azure DevOps Services pull request status watching.
use super::{BaseState, CheckResult, Clock, Failure, GRACE, Options, Outcome, POLL, Waited};
use crate::pr_host::{AzurePr, ToolProblem, Tools, az_json, encode_segment, required};
use serde_json::Value;
use std::collections::BTreeMap;

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
    tools: &dyn Tools,
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
    let current = pr.live_head(tools, data)?;
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
    tools: &dyn Tools,
    clock: &dyn Clock,
    activity: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(&str),
) -> Result<Waited, ToolProblem> {
    let pr = AzurePr::parse(url)?;
    let first = pr.show(tools)?;
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
    let head = pr.live_head(tools, &first)?;
    activity("Waiting for CI");
    let started = clock.elapsed();
    loop {
        let data = pr.show(tools)?;
        if let Some(waited) = moved_on(&pr, tools, &data, url, &head, activity)? {
            return Ok(waited);
        }
        let (checks, applicable) = checks_for(&pr, tools, &data, &head, url)?;
        let after = pr.show(tools)?;
        if let Some(waited) = moved_on(&pr, tools, &after, url, &head, activity)? {
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
