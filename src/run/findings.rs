//! The Findings part of a `jidoka run update`: new Findings and outcomes for recorded ones.

use super::{Finding, FindingState, MAX_TEXT_BYTES, Run, Stage, check_text};
use serde::{Deserialize, Serialize};

/// The security review runs in Check outside the check catalog, so its Findings name it separately.
const SECURITY_FINDING_CHECK: &str = "security";

/// Findings one update may record; matches `MAX_REPORTS_PER_UPDATE`.
const MAX_FINDINGS_PER_UPDATE: usize = 100;
/// Findings a run may hold in total, so a long run cannot grow its record without bound, like its reports.
const MAX_FINDINGS_PER_RUN: usize = 1000;

/// Help lines for the Findings flags, as a macro so `UPDATE_HELP` can stay a `const` built with `concat!`.
macro_rules! help {
    () => {
        concat!(
            "  --finding <check> <location|-> <text> (repeat; only in Check; records an issue a check reported as an open Finding, F1, F2, ...)\n",
            "  --finding-status <id>=<open|fixed|dismissed|deferred>[:<reason>] (repeat; dismissed and deferred need a reason; fixed also in Build during a Rework)\n"
        )
    };
}
pub(super) use help;

/// The checks a Finding may name: the catalog's checks and the security review.
fn finding_checks() -> impl Iterator<Item = &'static str> {
    crate::checks::CATALOG
        .iter()
        .map(|check| check.name)
        .chain(std::iter::once(SECURITY_FINDING_CHECK))
}

/// A finding as submitted by an update.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewFinding {
    pub(crate) check: String,
    #[serde(default)]
    pub(crate) location: Option<String>,
    pub(crate) text: String,
}

/// A new outcome for a recorded finding.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindingStatus {
    pub(crate) id: String,
    pub(crate) state: FindingState,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

/// What an update does to the run's Findings.
#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct FindingsUpdate {
    /// Findings to record; only in Check.
    #[serde(rename = "findings", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) new: Vec<NewFinding>,
    /// Outcomes for recorded findings.
    #[serde(
        rename = "finding_status",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(crate) statuses: Vec<FindingStatus>,
}

/// What `apply` needs to know about the update around the Findings.
pub(crate) struct FindingsContext<'a> {
    /// Where the update started.
    pub(crate) stage: Stage,
    /// Whether the update started a Rework.
    pub(crate) moved: bool,
    /// The update's timestamp, for new and resolved Findings.
    pub(crate) at: u64,
    pub(crate) git: &'a dyn crate::pr_sync::Git,
}

impl FindingsUpdate {
    /// Reads `--finding` or `--finding-status` and the values after it.
    pub(super) fn parse_option<'a>(
        &mut self,
        flag: &str,
        next_value: &mut dyn FnMut() -> Result<&'a &'a str, String>,
    ) -> Result<(), String> {
        if flag == "--finding" {
            let check = *next_value()?;
            let location = *next_value()?;
            let text = *next_value()?;
            self.new.push(NewFinding {
                check: check.to_string(),
                location: (location != "-").then(|| location.to_string()),
                text: text.to_string(),
            });
            return Ok(());
        }
        let value = *next_value()?;
        let bad = || {
            format!(
                "--finding-status needs <id>=<open|fixed|dismissed|deferred>[:<reason>], got {value:?}"
            )
        };
        let (id, rest) = value.split_once('=').ok_or_else(bad)?;
        let (state, reason) = match rest.split_once(':') {
            Some((state, reason)) => (state, Some(reason.to_string())),
            None => (rest, None),
        };
        let state = serde_json::from_value(state.into()).map_err(|_| bad())?;
        if self.statuses.iter().any(|known| known.id == id) {
            return Err(format!("Finding status for {id} given twice"));
        }
        self.statuses.push(FindingStatus {
            id: id.to_string(),
            state,
            reason,
        });
        Ok(())
    }

    /// Checks the limits, check names, texts and outcome reasons before anything is applied.
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.new.len() > MAX_FINDINGS_PER_UPDATE {
            return Err(format!(
                "At most {MAX_FINDINGS_PER_UPDATE} findings can be recorded at once"
            ));
        }
        for finding in &self.new {
            if !finding_checks().any(|name| name == finding.check) {
                let names: Vec<_> = finding_checks().collect();
                return Err(format!(
                    "Unknown check {:?} for --finding; use one of: {}",
                    finding.check,
                    names.join(", ")
                ));
            }
            check_text("Finding text", &finding.text, MAX_TEXT_BYTES)?;
            if let Some(location) = &finding.location {
                check_text("Finding location", location, MAX_TEXT_BYTES)?;
            }
        }
        for status in &self.statuses {
            match (status.state, status.reason.as_deref()) {
                (FindingState::Dismissed | FindingState::Deferred, reason) => {
                    check_text(
                        &format!("The reason for {} ({:?})", status.id, status.state),
                        reason.unwrap_or(""),
                        MAX_TEXT_BYTES,
                    )?;
                }
                (_, Some(_)) => {
                    return Err(format!(
                        "Finding {} cannot take a reason in state {:?}; only dismissed and deferred do",
                        status.id, status.state
                    ));
                }
                (_, None) => {}
            }
        }
        Ok(())
    }

    /// A Rework is the reason to go back to Build, so it needs a new Finding.
    pub(super) fn check_rework(&self, starts_rework: bool) -> Result<(), String> {
        if starts_rework && self.new.is_empty() {
            return Err(
                "A Rework needs a reason: add at least one --finding <check> <location|-> \"<text>\" to this update"
                    .into(),
            );
        }
        Ok(())
    }

    /// Records the new Findings and applies the outcomes.
    pub(super) fn apply(self, run: &mut Run, ctx: &FindingsContext) -> Result<(), String> {
        let FindingsContext {
            stage,
            moved,
            at,
            git,
        } = *ctx;
        let head = |run: &Run| crate::run_gate::head(run, git);
        if !self.new.is_empty() {
            if stage != Stage::Check {
                return Err(format!(
                    "Findings are recorded in Check; the run is in {}",
                    stage.name()
                ));
            }
            if run.findings.len() + self.new.len() > MAX_FINDINGS_PER_RUN {
                return Err(format!(
                    "A run can hold at most {MAX_FINDINGS_PER_RUN} findings"
                ));
            }
            let reviewed_commit = head(run)?;
            let mut number = run
                .findings
                .iter()
                .filter_map(|finding| finding.id.strip_prefix('F')?.parse::<u32>().ok())
                .max()
                .unwrap_or(0);
            for finding in self.new {
                number += 1;
                run.findings.push(Finding {
                    id: format!("F{number}"),
                    check: finding.check,
                    location: finding.location,
                    text: finding.text,
                    reviewed_commit: reviewed_commit.clone(),
                    rework: if moved { run.rework } else { None },
                    state: FindingState::Open,
                    reason: None,
                    fix_commit: None,
                    at,
                    resolved_at: None,
                });
            }
        }
        if moved {
            // Open Findings recorded earlier in this Check pass sent the run back too.
            for finding in run
                .findings
                .iter_mut()
                .filter(|finding| finding.state == FindingState::Open && finding.rework.is_none())
            {
                finding.rework = run.rework;
            }
        }
        if self.statuses.is_empty() {
            return Ok(());
        }
        let reworking = stage == Stage::Build && run.rework.is_some();
        if stage != Stage::Check && !reworking {
            return Err(format!(
                "Finding outcomes are set in Check, or in Build during a Rework; the run is in {}",
                stage.name()
            ));
        }
        let fix_commit = if self
            .statuses
            .iter()
            .any(|status| status.state == FindingState::Fixed)
        {
            Some(head(run)?)
        } else {
            None
        };
        for status in self.statuses {
            if stage == Stage::Build && status.state != FindingState::Fixed {
                return Err(format!(
                    "In Build only fixed can be set; {} is for Check",
                    status.id
                ));
            }
            let finding = run
                .findings
                .iter_mut()
                .find(|finding| finding.id == status.id)
                .ok_or_else(|| format!("No finding {} is recorded on this run", status.id))?;
            finding.state = status.state;
            finding.reason = status.reason;
            finding.resolved_at = (status.state != FindingState::Open).then_some(at);
            finding.fix_commit = if status.state == FindingState::Fixed {
                fix_commit.clone()
            } else {
                None
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{NewRun, RunStore, parse_update};
    use crate::test_support::TestDir;
    use std::path::Path;

    struct FixedGit;
    impl crate::pr_sync::Git for FixedGit {
        fn git(&self, _: &Path, _: &[&str]) -> Result<String, String> {
            Ok("abc1234".into())
        }
    }

    fn run_in(stage: Stage) -> Run {
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let mut run = store
            .create(NewRun {
                title: "task".into(),
                project: "/work/repo".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        run.stage = stage;
        run.worktree = Some("/work/wt".into());
        run
    }

    fn parsed(options: &[&str]) -> FindingsUpdate {
        parse_update("running", options).unwrap().findings
    }

    fn validated(options: &[&str]) -> Result<(), String> {
        parse_update("running", options)?.findings.validate()
    }

    fn apply(
        update: FindingsUpdate,
        run: &mut Run,
        stage: Stage,
        moved: bool,
    ) -> Result<(), String> {
        update.apply(
            run,
            &FindingsContext {
                stage,
                moved,
                at: 7,
                git: &FixedGit,
            },
        )
    }

    fn finding(id: &str) -> Finding {
        Finding {
            id: id.into(),
            check: "security".into(),
            location: None,
            text: "Leak".into(),
            reviewed_commit: "abc1234".into(),
            rework: None,
            state: FindingState::Open,
            reason: None,
            fix_commit: None,
            at: 1,
            resolved_at: None,
        }
    }

    #[test]
    fn findings_are_only_recorded_in_check() {
        let mut run = run_in(Stage::Build);
        let update = parsed(&["--finding", "security", "-", "Leak"]);
        let error = apply(update, &mut run, Stage::Build, false).unwrap_err();
        assert!(error.contains("recorded in Check"));
    }

    #[test]
    fn ids_continue_after_existing_ones() {
        let mut run = run_in(Stage::Check);
        run.findings = vec![finding("F1"), finding("F4")];
        let update = parsed(&["--finding", "security", "src/a.rs:1", "More"]);
        apply(update, &mut run, Stage::Check, false).unwrap();
        let added = &run.findings[2];
        assert_eq!(added.id, "F5");
        assert_eq!(added.location.as_deref(), Some("src/a.rs:1"));
        assert_eq!(added.reviewed_commit, "abc1234");
    }

    #[test]
    fn limits_apply_per_update_and_per_run() {
        let new = |count: usize| FindingsUpdate {
            new: (0..count)
                .map(|_| NewFinding {
                    check: "security".into(),
                    location: None,
                    text: "Leak".into(),
                })
                .collect(),
            statuses: Vec::new(),
        };
        assert!(new(MAX_FINDINGS_PER_UPDATE).validate().is_ok());
        assert!(new(MAX_FINDINGS_PER_UPDATE + 1).validate().is_err());

        let mut run = run_in(Stage::Check);
        run.findings = (1..=MAX_FINDINGS_PER_RUN)
            .map(|number| finding(&format!("F{number}")))
            .collect();
        let error = apply(new(1), &mut run, Stage::Check, false).unwrap_err();
        assert!(error.contains("at most"));
    }

    #[test]
    fn validate_refuses_unknown_checks_and_bad_reasons() {
        assert!(validated(&["--finding", "nope", "-", "Bug"]).is_err());
        assert!(validated(&["--finding", "security", "-", "Leak"]).is_ok());
        assert!(validated(&["--finding-status", "F1=dismissed"]).is_err());
        assert!(validated(&["--finding-status", "F1=deferred"]).is_err());
        assert!(validated(&["--finding-status", "F1=dismissed:fine"]).is_ok());
        assert!(validated(&["--finding-status", "F1=deferred:later"]).is_ok());
        assert!(validated(&["--finding-status", "F1=fixed:why"]).is_err());
        assert!(validated(&["--finding-status", "F1=open:why"]).is_err());
        assert!(validated(&["--finding-status", "F1=fixed"]).is_ok());
    }

    #[test]
    fn a_rework_needs_a_new_finding() {
        assert!(parsed(&[]).check_rework(true).is_err());
        assert!(parsed(&[]).check_rework(false).is_ok());
        let update = parsed(&["--finding", "security", "-", "Leak"]);
        assert!(update.check_rework(true).is_ok());
    }

    #[test]
    fn build_accepts_only_fixed_during_a_rework() {
        let mut run = run_in(Stage::Build);
        run.findings = vec![finding("F1")];
        let fixed = || parsed(&["--finding-status", "F1=fixed"]);
        let error = apply(fixed(), &mut run, Stage::Build, false).unwrap_err();
        assert!(error.contains("outcomes are set in Check"));

        run.rework = Some(1);
        let deferred = parsed(&["--finding-status", "F1=deferred:later"]);
        let error = apply(deferred, &mut run, Stage::Build, false).unwrap_err();
        assert!(error.contains("only fixed"));

        apply(fixed(), &mut run, Stage::Build, false).unwrap();
        assert_eq!(run.findings[0].state, FindingState::Fixed);
        assert_eq!(run.findings[0].fix_commit.as_deref(), Some("abc1234"));
        assert_eq!(run.findings[0].resolved_at, Some(7));
    }

    #[test]
    fn outcomes_in_check_record_reason_and_unknown_ids_fail() {
        let mut run = run_in(Stage::Check);
        run.findings = vec![finding("F1")];
        let missing = parsed(&["--finding-status", "F9=fixed"]);
        assert!(apply(missing, &mut run, Stage::Check, false).is_err());
        let deferred = parsed(&["--finding-status", "F1=deferred:not worth it"]);
        apply(deferred, &mut run, Stage::Check, false).unwrap();
        assert_eq!(run.findings[0].reason.as_deref(), Some("not worth it"));
        assert_eq!(run.findings[0].fix_commit, None);
    }

    #[test]
    fn a_rework_tags_earlier_open_findings_but_not_resolved_ones() {
        let mut run = run_in(Stage::Check);
        run.rework = Some(1);
        let mut fixed = finding("F2");
        fixed.state = FindingState::Fixed;
        run.findings = vec![finding("F1"), fixed];
        let update = parsed(&["--finding", "security", "-", "Another"]);
        apply(update, &mut run, Stage::Check, true).unwrap();
        let tagged: Vec<_> = run.findings.iter().map(|f| f.rework).collect();
        assert_eq!(tagged, [Some(1), None, Some(1)]);
    }

    #[test]
    fn json_keeps_its_field_names_and_rejects_unknown_keys() {
        let update: crate::run::UpdateRun = serde_json::from_str(
            r#"{"status":"running","findings":[{"check":"security","text":"Leak"}],"finding_status":[{"id":"F1","state":"open"}]}"#,
        )
        .unwrap();
        assert_eq!(update.findings.new.len(), 1);
        assert_eq!(update.findings.statuses.len(), 1);
        let json = serde_json::to_value(&update).unwrap();
        assert!(json["findings"].is_array() && json["finding_status"].is_array());
        let unknown =
            serde_json::from_str::<crate::run::UpdateRun>(r#"{"status":"running","bogus":1}"#);
        assert!(unknown.is_err());
    }
}
