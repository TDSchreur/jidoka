//! Model routing: which model and effort each Stage should use, recorded on a run when it starts.
//! "Routing" alone already means workflow selection in this repository, so this is always model routing.
//! The record holds only the per-Stage models of the agent the run started in. The coordinating session
//! is not routed.

use crate::run::Stage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod policy;
pub(crate) use policy::{
    install_starter, load, parse_start_options, policy_path_in, read, Coverage, Host, Policy,
    Resolution, Starter,
};

/// The coding agent a selection is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RoutingAgent {
    Codex,
    Claude,
    Copilot,
}

/// A Stage's model and effort. Both are free text: the factory keeps no list of valid models. Copilot CLI
/// entries get a few extra checks (see `validate_for_copilot`), because it ignores what it does not know.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelChoice {
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
}

/// The selection recorded on a run.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelRouting {
    pub(crate) agent: RoutingAgent,
    /// The policy's model per Stage, as it was when the run started. A Stage left out is not routed.
    pub(crate) stages: BTreeMap<Stage, ModelChoice>,
    /// Choices made for this run by hand; they win over `stages` and leave it untouched.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) overrides: BTreeMap<Stage, ModelChoice>,
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
    /// The model for `stage`: this run's override if it has one, otherwise the policy's choice.
    pub(crate) fn choice_for(&self, stage: Stage) -> Option<&ModelChoice> {
        self.overrides
            .get(&stage)
            .or_else(|| self.stages.get(&stage))
    }

    /// The rules every selection follows, whether the CLI resolved it or the API received it.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for choice in self.stages.values().chain(self.overrides.values()) {
            choice.validate()?;
            if self.agent == RoutingAgent::Copilot {
                choice.validate_for_copilot()?;
            }
        }
        Ok(())
    }
}

impl ModelChoice {
    fn validate(&self) -> Result<(), String> {
        check_text("Model", &self.model)?;
        if let Some(effort) = &self.effort {
            check_text("Effort", effort)?;
        }
        Ok(())
    }

    /// Parses `<model>[:<effort>]`, as given to `--model-routing`.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let (model, effort) = match text.split_once(':') {
            Some((model, effort)) => (model, Some(effort)),
            None => (text, None),
        };
        let choice = Self {
            model: model.to_string(),
            effort: effort.map(str::to_string),
        };
        choice.validate()?;
        Ok(choice)
    }
}

impl RoutingAgent {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Copilot => "copilot",
        }
    }
}

/// The reasoning efforts Copilot CLI lists for `--reasoning-effort`; any other text is silently ignored.
const COPILOT_EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
/// Claude Code's subagent aliases. Copilot CLI takes exact model IDs and quietly runs an unknown name on the session's model.
const CLAUDE_CODE_ALIASES: [&str; 4] = ["opus", "sonnet", "haiku", "fable"];

impl ModelChoice {
    /// What Copilot CLI needs on top of the shared rules: a model ID, not a Claude Code alias, and an effort it lists.
    fn validate_for_copilot(&self) -> Result<(), String> {
        let alias = self.model.to_ascii_lowercase();
        if CLAUDE_CODE_ALIASES.contains(&alias.as_str()) {
            return Err(format!(
                "Model {:?} is a Claude Code alias; Copilot CLI needs an exact model ID such as claude-sonnet-5.5, and runs an unknown name on the session's model",
                self.model
            ));
        }
        match self.effort.as_deref() {
            Some(effort) if !COPILOT_EFFORTS.contains(&effort) => Err(format!(
                "Effort {effort:?} is not one Copilot CLI accepts ({})",
                COPILOT_EFFORTS.join(", ")
            )),
            _ => Ok(()),
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
