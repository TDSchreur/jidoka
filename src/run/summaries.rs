//! The stage summaries of a `jidoka run update`.

use super::{MAX_NOTE_BYTES, Run, Stage, check_text, split_pair};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Help lines for the summary flags, as a macro so `UPDATE_HELP` can stay a `const` built with `concat!`.
macro_rules! help {
    () => {
        "  --summary <define|build|check|finish>=<text> | --clear-summary <stage> (repeat; `done` is only accepted in Finish and needs a Finish summary)\n"
    };
}
pub(super) use help;

/// What an update does to the run's stage summaries.
#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct SummariesUpdate {
    /// Stage summaries to set; null removes that stage's summary. Other stages keep theirs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    summaries: BTreeMap<Stage, Option<String>>,
}

impl SummariesUpdate {
    /// Sets the summary of `stage`; `None` removes it.
    pub(crate) fn set(&mut self, stage: Stage, summary: Option<String>) {
        self.summaries.insert(stage, summary);
    }

    /// Reads `--summary` or `--clear-summary` and the value after it.
    pub(super) fn parse_option<'a>(
        &mut self,
        flag: &str,
        next_value: &mut dyn FnMut() -> Result<&'a &'a str, String>,
    ) -> Result<(), String> {
        let value = *next_value()?;
        let (name, text) = if flag == "--summary" {
            let (name, text) = split_pair(flag, value)?;
            (name, Some(text.to_string()))
        } else {
            (value, None)
        };
        let stage: Stage = serde_json::from_value(name.into()).map_err(|_| {
            format!("Unknown stage {name:?} for {flag}; use define, build, check or finish")
        })?;
        if self.summaries.insert(stage, text).is_some() {
            return Err(format!("Summary for {name} given twice"));
        }
        Ok(())
    }

    /// Checks every summary text before anything is applied.
    pub(super) fn validate(&self) -> Result<(), String> {
        for summary in self.summaries.values().flatten() {
            check_text("Stage summary", summary, MAX_NOTE_BYTES)?;
        }
        Ok(())
    }

    /// Sets and removes the summaries; other stages keep theirs.
    pub(super) fn apply(self, run: &mut Run) {
        for (stage, summary) in self.summaries {
            match summary {
                Some(text) => {
                    run.summaries.insert(stage, text);
                }
                None => {
                    run.summaries.remove(&stage);
                }
            }
        }
    }
}

/// A run is marked done only with a Finish summary, so a reader can tell what shipped.
pub(super) fn check_done(run: &Run) -> Result<(), String> {
    if run
        .summaries
        .get(&Stage::Finish)
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err(
            "Finish cannot be marked done without a Finish summary; add --summary finish=<what shipped>"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{parse_update, tests::sample_run};

    fn parsed(options: &[&str]) -> Result<SummariesUpdate, String> {
        Ok(parse_update("running", options)?.summaries)
    }

    #[test]
    fn a_summary_is_set_or_removed_and_other_stages_are_kept() {
        let mut run = sample_run();
        run.summaries.insert(Stage::Define, "Planned".into());
        run.summaries.insert(Stage::Build, "Built".into());
        parsed(&["--summary", "check=Fine", "--clear-summary", "build"])
            .unwrap()
            .apply(&mut run);
        assert_eq!(run.summaries.len(), 2);
        assert_eq!(run.summaries[&Stage::Check], "Fine");
        assert!(run.summaries.contains_key(&Stage::Define));
    }

    #[test]
    fn done_needs_a_nonblank_finish_summary() {
        let mut run = sample_run();
        assert!(check_done(&run).is_err());
        run.summaries.insert(Stage::Finish, "  ".into());
        assert!(check_done(&run).is_err());
        run.summaries.insert(Stage::Finish, "Shipped".into());
        assert!(check_done(&run).is_ok());
    }

    #[test]
    fn a_stage_is_given_once_and_must_exist() {
        assert!(parsed(&["--summary", "build=a", "--clear-summary", "build"]).is_err());
        assert!(parsed(&["--summary", "deploy=a"]).is_err());
    }
}
