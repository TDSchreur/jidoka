//! The model-routing section of `factory doctor`: whether the user's policy can work on this machine.
//! Only the policy file itself can make the report not ready, because only it stops `factory run start`.
//! Everything else is a warning or skipped: model routing is optional, and without it a stage uses the
//! session's model. Nothing here changes a file or calls a model; at most it asks an agent for its `--version`.

use super::{find_program, read_doc, Check, Machine};
use crate::routing::{policy_path_in, read, Coverage, Host, ModelRouting, Policy, RoutingAgent};
use std::path::Path;

/// The Agent tool's `effort` option first appeared in this Claude Code release.
const CLAUDE_MIN_VERSION: (u32, u32, u32) = (2, 1, 292);
/// The only model names the Agent tool accepts.
const CLAUDE_MODELS: [&str; 4] = ["opus", "sonnet", "haiku", "fable"];
const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// The efforts Copilot CLI lists for `--reasoning-effort`.
const COPILOT_EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// The routing checks for `agent` (all agents when `None`), as `--agent` on `factory doctor` selects. What
/// the policy does for each agent comes from `Policy::coverage`, the same rule `factory run start` follows;
/// this only adds what the machine needs.
pub(super) fn checks(root: &Path, machine: &Machine, agent: Option<&str>) -> Vec<Check> {
    let Some(path) = policy_path_in(|name| (machine.var)(name)) else {
        return Vec::new();
    };
    let policy = match read(&path) {
        Ok(policy) => policy,
        Err(problem) => {
            return vec![Check::fail(
                "policy",
                problem,
                "Correct the file, or set \"enabled\": false to switch model routing off.",
            )]
        }
    };
    let shown = path.display();
    // The agents to report on: the one asked for, or those the policy is about.
    let hosts: Vec<Host> = match (agent.and_then(Host::parse), &policy) {
        (_, Policy::Off) => return Vec::new(),
        (Some(host), _) => vec![host],
        (None, Policy::Multi { .. }) => policy
            .configured()
            .into_iter()
            .filter_map(Host::parse)
            .collect(),
    };
    // Every check says which agent it is about.
    let mut checks = vec![Check::pass(
        "policy",
        format!("{shown} configures {}", policy.configured().join(", ")),
    )];
    for host in hosts {
        let name = host.name();
        let tag = |check: Check| check.for_agent(name);
        match policy.coverage(host) {
            None => {}
            Some(Coverage::Routed(selection)) => {
                checks.extend(agent_checks(&selection, root, machine).into_iter().map(tag));
            }
            Some(Coverage::NotRouted(why)) => {
                checks.push(tag(Check::skipped("policy", why.text(host))));
            }
        }
    }
    checks
}

fn agent_checks(selection: &ModelRouting, root: &Path, machine: &Machine) -> Vec<Check> {
    match selection.agent {
        RoutingAgent::Claude => claude_checks(selection, root, machine),
        RoutingAgent::Codex => codex_checks(machine),
        RoutingAgent::Copilot => copilot_checks(selection, machine),
    }
}

/// `2.1.318 (Claude Code)` is `(2, 1, 318)`.
pub(super) fn parse_version(output: &str) -> Option<(u32, u32, u32)> {
    output.split_whitespace().find_map(|word| {
        let mut parts = word.trim_start_matches('v').split('.');
        let mut next = || parts.next()?.parse().ok();
        Some((next()?, next()?, next()?))
    })
}

fn version_text((major, minor, patch): (u32, u32, u32)) -> String {
    format!("{major}.{minor}.{patch}")
}

fn host_check(agent: &'static str, machine: &Machine, minimum: Option<(u32, u32, u32)>) -> Check {
    let Some(program) = find_program(agent, &machine.path) else {
        return Check::warning(
            "host",
            format!("`{agent}` is not on PATH, so model routing cannot be applied and every stage uses the session's model"),
            format!("Install the {agent} command line tool and make sure `{agent}` is on PATH."),
        );
    };
    let version = (machine.version)(&program).and_then(|output| parse_version(&output));
    match (version, minimum) {
        (None, _) => Check::skipped(
            "host",
            format!("could not read the version of `{agent}`, so support for model routing was not checked"),
        ),
        (Some(found), Some(minimum)) if found < minimum => Check::warning(
            "host",
            format!(
                "`{agent}` {} is older than {}, the first release with the Agent tool's effort option",
                version_text(found),
                version_text(minimum)
            ),
            format!("Update {agent} to {} or later.", version_text(minimum)),
        ),
        (Some(found), Some(_)) => Check::pass(
            "host",
            format!("`{agent}` {} supports model routing", version_text(found)),
        ),
        (Some(found), None) => Check::skipped(
            "host",
            format!(
                "`{agent}` {} found; the factory does not know which release first supports a model and effort per subagent, so the version was not checked",
                version_text(found)
            ),
        ),
    }
}

fn codex_checks(machine: &Machine) -> Vec<Check> {
    vec![
        host_check("codex", machine, None),
        Check::skipped(
            "model",
            "Codex models and efforts are not checked: the factory keeps no list of them",
        ),
        Check::skipped("overrides", "Codex configuration is not checked"),
    ]
}

fn copilot_checks(selection: &ModelRouting, machine: &Machine) -> Vec<Check> {
    vec![
        host_check("copilot", machine, None),
        copilot_model_check(selection),
        effort_check(
            selection,
            "Copilot CLI",
            &COPILOT_EFFORTS,
            "Set effort to none, minimal, low, medium, high, xhigh or max for each routed stage.",
        ),
        Check::skipped("overrides", "Copilot configuration is not checked"),
    ]
}

/// The policy check already rejects Claude Code aliases, which Copilot CLI would run on the session's model.
/// Which models an account can use is not known to the factory, so availability is left to the user.
fn copilot_model_check(selection: &ModelRouting) -> Check {
    let count = selection.stages.len();
    Check::pass(
        "model",
        format!(
            "{count} routed stage(s) name an exact model ID; whether the account can use them is not checked, so compare the models `factory run inspect` observed"
        ),
    )
}

fn claude_checks(selection: &ModelRouting, root: &Path, machine: &Machine) -> Vec<Check> {
    vec![
        host_check("claude", machine, Some(CLAUDE_MIN_VERSION)),
        claude_model_check(selection),
        effort_check(
            selection,
            "Claude Code",
            &CLAUDE_EFFORTS,
            "Set effort to low, medium, high, xhigh or max for each routed stage.",
        ),
        claude_overrides_check(root, machine),
    ]
}

fn claude_model_check(selection: &ModelRouting) -> Check {
    let wrong: Vec<String> = selection
        .stages
        .iter()
        .filter_map(|(stage, choice)| {
            let model = &choice.model;
            (!CLAUDE_MODELS.contains(&model.as_str())).then(|| format!("{}: {model}", stage.name()))
        })
        .collect();
    if wrong.is_empty() {
        return Check::pass(
            "model",
            "every routed stage uses a model the Agent tool accepts",
        );
    }
    Check::warning(
        "model",
        format!(
            "the Agent tool accepts only {}, not: {}",
            CLAUDE_MODELS.join(", "),
            wrong.join(", ")
        ),
        "Use opus, sonnet, haiku or fable for these stages.",
    )
}

fn effort_check(selection: &ModelRouting, host: &str, accepted: &[&str], fix: &str) -> Check {
    let (mut unsupported, mut unset) = (Vec::new(), Vec::new());
    for (stage, choice) in &selection.stages {
        match choice.effort.as_deref() {
            None => unset.push(stage.name().to_string()),
            Some(effort) if !accepted.contains(&effort) => {
                unsupported.push(format!("{}: {effort}", stage.name()));
            }
            Some(_) => {}
        }
    }
    let mut problems = Vec::new();
    if !unsupported.is_empty() {
        problems.push(format!(
            "unsupported effort ({}): {}",
            accepted.join(", "),
            unsupported.join(", ")
        ));
    }
    if !unset.is_empty() {
        problems.push(format!(
            "no effort set, so the model's own default applies, and defaults differ between models: {}",
            unset.join(", ")
        ));
    }
    if problems.is_empty() {
        return Check::pass(
            "effort",
            format!("every routed stage has an effort {host} accepts; which levels a model offers is not checked"),
        );
    }
    Check::warning("effort", problems.join("; "), fix)
}

/// Where Claude Code settings can set environment variables, besides the environment itself.
fn settings_files(root: &Path, machine: &Machine) -> Vec<(String, std::path::PathBuf)> {
    let mut files = Vec::new();
    if let Ok(home) = crate::install::agent_home("claude", &machine.home, machine.var) {
        files.push(("user settings".to_string(), home.join("settings.json")));
    }
    files.push((
        "project settings".into(),
        root.join(".claude/settings.json"),
    ));
    files.push((
        "local project settings".into(),
        root.join(".claude/settings.local.json"),
    ));
    files
}

fn claude_overrides_check(root: &Path, machine: &Machine) -> Check {
    const NAMES: [&str; 3] = [
        "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
        "CLAUDE_CODE_SUBAGENT_MODEL",
        "CLAUDE_CODE_EFFORT_LEVEL",
    ];
    // (source, name, value), environment first.
    let mut found: Vec<(String, &str, String)> = Vec::new();
    let mut unreadable = Vec::new();
    for name in NAMES {
        if let Some(value) = (machine.var)(name).filter(|v| !v.is_empty()) {
            found.push((
                "the environment".into(),
                name,
                value.to_string_lossy().into_owned(),
            ));
        }
    }
    for (source, path) in settings_files(root, machine) {
        let text = match read_doc(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                unreadable.push(path.display().to_string());
                continue;
            }
        };
        let Ok(settings) = serde_json::from_str::<serde_json::Value>(&text) else {
            unreadable.push(path.display().to_string());
            continue;
        };
        for name in NAMES {
            if let Some(value) = settings["env"][name].as_str().filter(|v| !v.is_empty()) {
                found.push((source.clone(), name, value.to_string()));
            }
        }
    }
    let value_of = |name: &str| found.iter().find(|(_, n, _)| *n == name);
    let mut overrides = Vec::new();
    if let (Some((_, _, force)), Some((source, _, model))) = (
        value_of("CLAUDE_CODE_SUBAGENT_MODEL_FORCE"),
        value_of("CLAUDE_CODE_SUBAGENT_MODEL"),
    ) {
        if force == "1" {
            overrides.push(format!(
                "CLAUDE_CODE_SUBAGENT_MODEL_FORCE=1 with CLAUDE_CODE_SUBAGENT_MODEL={model} ({source}) puts every subagent on {model}, replacing the policy's models"
            ));
        }
    }
    if let Some((source, _, level)) = value_of("CLAUDE_CODE_EFFORT_LEVEL") {
        overrides.push(format!(
            "CLAUDE_CODE_EFFORT_LEVEL={level} ({source}) replaces the policy's efforts"
        ));
    }
    if !overrides.is_empty() {
        return Check::warning(
            "overrides",
            overrides.join("; "),
            "Unset these variables, or remove them from the settings files, while you use model routing.",
        );
    }
    if !unreadable.is_empty() {
        return Check::skipped(
            "overrides",
            format!(
                "could not read {}, so overrides set there were not checked",
                unreadable.join(", ")
            ),
        );
    }
    Check::pass(
        "overrides",
        "nothing in the environment or the Claude Code settings overrides the policy",
    )
}
