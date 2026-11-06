//! Model routing: which model and effort each kind of task should use, recorded on a run when it starts.
//! "Routing" alone already means workflow selection in this repository, so this is always model routing.
//! The record holds only the roles and the task mapping of the agent the run started in. Only the review
//! agents that Check starts use it; Build, Finish and the coordinating session are not routed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod policy;
#[cfg(test)]
pub(crate) use policy::Host;
pub(crate) use policy::{
    create_starter, load, parse_start_options, policy_path_in, read, AgentPolicy, Policy,
    Resolution,
};

/// The coding agent a selection is for. Copilot CLI is not here: it records no selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RoutingAgent {
    Codex,
    Claude,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    Strong,
    Coding,
    Routine,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Task {
    Build,
    Check,
    Finish,
}

/// A role's model and effort. Both are free text: the factory keeps no list of valid values.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoleChoice {
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
}

/// The selection recorded on a run.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelRouting {
    pub(crate) agent: RoutingAgent,
    pub(crate) roles: BTreeMap<Role, RoleChoice>,
    /// The policy's mapping of tasks to roles, as it was when the run started.
    pub(crate) tasks: BTreeMap<Task, Role>,
    /// Roles chosen for this run by hand; they win over `tasks` and leave it untouched.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) overrides: BTreeMap<Task, Role>,
}

/// Whether an observed model id is the model that was requested. A requested alias is a whole part of the
/// id (`sonnet` in `claude-sonnet-5-5`); a version is not cut (`gpt-6` is not in `gpt-6.1-sol`).
pub(crate) fn model_matches(requested: &str, observed: &str) -> bool {
    let (requested, observed) = (
        requested.to_ascii_lowercase(),
        observed.to_ascii_lowercase(),
    );
    let boundary = |c: Option<char>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '.'));
    !requested.is_empty()
        && observed.match_indices(&requested).any(|(start, _)| {
            boundary(observed[..start].chars().next_back())
                && boundary(observed[start + requested.len()..].chars().next())
        })
}

/// Longest model or effort text. They are shown in a terminal and handed to agents, so they stay short.
const MAX_TEXT: usize = 64;

fn check_text(name: &str, value: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-');
    if value.is_empty() || value.len() > MAX_TEXT || !value.chars().all(allowed) {
        return Err(format!(
            "{name} must be 1 to {MAX_TEXT} characters from letters, digits and . _ : / -"
        ));
    }
    Ok(())
}

impl ModelRouting {
    /// The role that handles `task`: this run's override if it has one, otherwise the policy's mapping.
    pub(crate) fn role_for(&self, task: Task) -> Option<Role> {
        self.overrides
            .get(&task)
            .or_else(|| self.tasks.get(&task))
            .copied()
    }

    /// The rules every selection follows, whether the CLI resolved it or the API received it.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for choice in self.roles.values() {
            check_text("Model", &choice.model)?;
            if let Some(effort) = &choice.effort {
                check_text("Effort", effort)?;
            }
        }
        for (task, role) in self.tasks.iter().chain(&self.overrides) {
            if !self.roles.contains_key(role) {
                return Err(format!(
                    "The {} task uses the {} role, which is not defined under roles",
                    task.name(),
                    role.name()
                ));
            }
        }
        Ok(())
    }
}

impl RoutingAgent {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

impl Role {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Coding => "coding",
            Self::Routine => "routine",
        }
    }
}

impl Task {
    pub(crate) const ALL: [Task; 3] = [Task::Build, Task::Check, Task::Finish];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Check => "check",
            Self::Finish => "finish",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requested_model_matches_an_observed_id_by_whole_parts() {
        for (requested, observed) in [
            ("sonnet", "claude-sonnet-5-5"),
            ("opus", "claude-opus-5-5"),
            ("OPUS", "claude-opus-4-8"),
            ("haiku", "claude-haiku-5-5"),
            ("gpt-6.1-sol", "gpt-6.1-sol"),
            ("gpt-6-astra", "gpt-6-astra"),
        ] {
            assert!(
                model_matches(requested, observed),
                "{requested} / {observed}"
            );
        }
        for (requested, observed) in [
            ("opus", "claude-sonnet-5-5"),
            ("gpt-6", "gpt-6.1-sol"),
            ("son", "claude-sonnet-5-5"),
            ("", "claude-sonnet-5-5"),
        ] {
            assert!(
                !model_matches(requested, observed),
                "{requested} / {observed}"
            );
        }
    }
}
