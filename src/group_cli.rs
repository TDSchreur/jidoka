//! `jidoka group ...`: reads and changes change groups through the dashboard's `/api/groups` endpoints.

use crate::{
    client::{api_get, api_post},
    groups::{Group, GroupView, Member},
    inspect::shown,
};
use std::path::Path;

type Failure = Box<dyn std::error::Error>;

/// Runs a `group` subcommand; `words` are the arguments after `group`.
pub(crate) fn run(words: &[&str], server: &str, token_file: &Path) -> Result<(), Failure> {
    let usage = || "Invalid arguments; see `jidoka --help`".to_string();
    match words {
        ["create", title, options @ ..] if !title.starts_with('-') => {
            let (spec, runs) = parse_create(options).ok_or_else(usage)?;
            let body =
                serde_json::json!({ "title": title, "spec": spec_reference(&spec)?, "runs": runs });
            let group: Group = post(server, token_file, "/api/groups", &body)?;
            println!("Created group {}", group.id);
        }
        ["list", options @ ..] => {
            let json = json_flag(options).ok_or_else(usage)?;
            let groups = crate::client::fetch_groups(server)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&groups)?);
            } else {
                print!("{}", render_list(&groups));
            }
        }
        ["inspect", id, options @ ..] => {
            let json = json_flag(options).ok_or_else(usage)?;
            let bytes = api_get(server, &format!("/api/groups/{id}"))?;
            if json {
                let view: serde_json::Value = serde_json::from_slice(&bytes)?;
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                let view: GroupView = serde_json::from_slice(&bytes)?;
                print!("{}", render_view(&view, crate::run::updated_now()?));
            }
        }
        ["add", id, run] => {
            let group: Group = post(
                server,
                token_file,
                &format!("/api/groups/{id}/add"),
                &serde_json::json!({ "run": run }),
            )?;
            println!("Added run {run} to group {}", group.id);
        }
        ["remove", id, run] => {
            let group: Group = post(
                server,
                token_file,
                &format!("/api/groups/{id}/remove"),
                &serde_json::json!({ "run": run }),
            )?;
            println!("Removed run {run} from group {}", group.id);
        }
        ["delete", id] => {
            api_post(server, token_file, &format!("/api/groups/{id}/delete"), b"")?;
            println!("Deleted group {id}");
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn post(
    server: &str,
    token_file: &Path,
    route: &str,
    body: &serde_json::Value,
) -> Result<Group, Failure> {
    let response = api_post(server, token_file, route, &serde_json::to_vec(body)?)?;
    Ok(serde_json::from_slice(&response)?)
}

fn json_flag(options: &[&str]) -> Option<bool> {
    match options {
        [] => Some(false),
        ["--json"] => Some(true),
        _ => None,
    }
}

/// The `--spec` value and the `--run` ids of `group create`.
fn parse_create(options: &[&str]) -> Option<(String, Vec<String>)> {
    let mut spec = None;
    let mut runs = Vec::new();
    let mut words = options.iter();
    while let Some(option) = words.next() {
        let value = words.next()?;
        match *option {
            "--spec" if spec.is_none() => spec = Some((*value).to_string()),
            "--run" => runs.push((*value).to_string()),
            _ => return None,
        }
    }
    Some((spec?, runs))
}

/// A web link stays as it is. A local file becomes its absolute path now, so later commands work from any
/// checkout.
fn spec_reference(spec: &str) -> Result<String, Failure> {
    let lower = spec.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(spec.to_string());
    }
    let absolute = std::path::absolute(spec)?;
    if !absolute.is_file() {
        return Err(format!("Spec {spec} is not an http(s) URL or an existing file").into());
    }
    Ok(absolute.to_string_lossy().into_owned())
}

fn render_list(groups: &[Group]) -> String {
    if groups.is_empty() {
        return "No groups\n".to_string();
    }
    groups
        .iter()
        .map(|group| {
            format!(
                "{}  {}  ({} runs)\n",
                group.id,
                shown(&group.title),
                group.runs.len()
            )
        })
        .collect()
}

fn render_view(view: &GroupView, now: u64) -> String {
    let mut out = format!("Group {}: {}\n", view.id, shown(&view.title));
    out.push_str(&format!("{:<14}{}\n", "Spec", shown(&view.spec)));
    out.push_str(&format!("{:<14}{}\n", "Summary", view.summary));
    if view.members.is_empty() {
        out.push_str("Runs          none attached\n");
    } else {
        out.push_str("Runs\n");
    }
    for member in &view.members {
        out.push_str(&render_member(member, now));
    }
    out
}

fn render_member(member: &Member, now: u64) -> String {
    if member.missing {
        return format!(
            "  {}  run missing; detach it with `jidoka group remove`\n",
            member.run
        );
    }
    let mut out = format!(
        "  {}  {}\n",
        member.run,
        shown(member.title.as_deref().unwrap_or_default())
    );
    if let Some(repository) = &member.repository {
        out.push_str(&format!("      Repository  {}\n", shown(repository)));
    }
    out.push_str(&format!(
        "      Pull request  {}\n",
        pull_request(member, now)
    ));
    out
}

fn pull_request(member: &Member, now: u64) -> String {
    let Some(url) = &member.pr else {
        return "none recorded".to_string();
    };
    let Some(status) = &member.pr_status else {
        return format!("{} (unchecked)", shown(url));
    };
    let state = status.state.label();
    let reason = status
        .reason
        .as_deref()
        .map_or(String::new(), |reason| format!(": {}", shown(reason)));
    format!(
        "{} ({state}{reason}, checked {})",
        shown(url),
        ago(now, status.checked_at)
    )
}

/// How long before `now` a time in milliseconds was.
fn ago(now: u64, then: u64) -> String {
    let seconds = now.saturating_sub(then) / 1000;
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86400),
    }
}

#[cfg(test)]
mod tests;
