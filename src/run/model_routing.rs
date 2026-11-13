//! The model-routing overrides of a `jidoka run update`.

use super::{FieldUpdate, Run, Stage, repeated, split_pair};
use crate::routing::ModelChoice;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Help lines for the routing flags, as a macro so `UPDATE_HELP` can stay a `const` built with `concat!`.
macro_rules! help {
    () => {
        "  --model-routing <define|build|check|finish>=<model>[:<effort>] (repeat; the flags are all of the run's overrides) | --clear-model-routing\n"
    };
}
pub(super) use help;

/// What an update does to the run's model-routing overrides.
#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct RoutingUpdate {
    /// The run's whole set of model-routing overrides; null removes them.
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) model_routing: FieldUpdate<BTreeMap<Stage, ModelChoice>>,
}

impl RoutingUpdate {
    /// Reads `--model-routing` or `--clear-model-routing` and the value after it.
    pub(super) fn parse_option<'a>(
        &mut self,
        flag: &str,
        next_value: &mut dyn FnMut() -> Result<&'a &'a str, String>,
    ) -> Result<(), String> {
        if flag == "--clear-model-routing" {
            if !self.model_routing.is_keep() {
                return Err(repeated(flag));
            }
            self.model_routing = FieldUpdate::Clear;
            return Ok(());
        }
        let value = next_value()?;
        let (name, choice) = split_pair(flag, value)?;
        let stage: Stage = serde_json::from_value(name.into()).map_err(|_| {
            format!(
                "Unknown stage {name:?} for --model-routing; use define, build, check or finish"
            )
        })?;
        let choice = ModelChoice::parse(choice)
            .map_err(|problem| format!("Invalid --model-routing for {name}: {problem}"))?;
        if self.model_routing.is_keep() {
            self.model_routing = FieldUpdate::Set(BTreeMap::new());
        }
        let FieldUpdate::Set(overrides) = &mut self.model_routing else {
            return Err(repeated(flag));
        };
        if overrides.insert(stage, choice).is_some() {
            return Err(format!("Model for {name} given twice"));
        }
        Ok(())
    }

    /// Replaces or clears the overrides of the run's routing selection.
    pub(super) fn apply(self, run: &mut Run) -> Result<(), String> {
        match self.model_routing {
            FieldUpdate::Keep => {}
            FieldUpdate::Clear => {
                if let Some(routing) = run.model_routing.as_mut() {
                    routing.overrides.clear();
                }
            }
            FieldUpdate::Set(overrides) => {
                let routing = run
                    .model_routing
                    .as_mut()
                    .ok_or("This run has no model routing selection to override")?;
                routing.overrides = overrides;
                routing.validate()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{parse_update, tests::sample_run};

    fn parsed(options: &[&str]) -> Result<RoutingUpdate, String> {
        Ok(parse_update("running", options)?.routing)
    }

    #[test]
    fn overrides_are_collected_and_repeats_or_conflicts_refused() {
        let FieldUpdate::Set(overrides) = parsed(&[
            "--model-routing",
            "build=opus",
            "--model-routing",
            "check=sonnet",
        ])
        .unwrap()
        .model_routing
        else {
            panic!("overrides were not set");
        };
        assert_eq!(overrides.len(), 2);
        assert!(
            parsed(&[
                "--model-routing",
                "build=opus",
                "--model-routing",
                "build=x"
            ])
            .is_err()
        );
        assert!(parsed(&["--clear-model-routing", "--model-routing", "build=opus"]).is_err());
        assert!(parsed(&["--model-routing", "deploy=opus"]).is_err());
    }

    #[test]
    fn overrides_need_a_routing_selection() {
        let update = parsed(&["--model-routing", "build=opus"]).unwrap();
        assert!(update.apply(&mut sample_run()).is_err());
    }
}
