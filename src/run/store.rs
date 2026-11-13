//! The run records and the one way to change them safely.

use super::*;
use crate::run_gate::GateResult;

/// The run records, shared by clones: usage is collected in the background on a clone.
#[derive(Clone)]
pub(crate) struct RunStore {
    shared: Arc<Shared>,
}

struct Shared {
    path: PathBuf,
    runs: Mutex<Vec<Run>>,
    collection: Collection,
}

/// Serializes the parts of a run that gates read.
///
/// Fields gates never read and background collectors write meanwhile (PR status, usage, run reports,
/// timestamps) are blanked, so they cannot make a slow gate check fail spuriously. `change_gate` keeps the
/// stored values of these fields when it saves.
fn gate_view(run: &Run) -> Result<Vec<u8>, String> {
    let mut view = run.clone();
    view.pr_status = None;
    view.usage = None;
    view.run_reports.clear();
    view.updated_at = None;
    view.agent_updated_at = None;
    serde_json::to_vec(&view).map_err(|e| e.to_string())
}

/// Who changed a run, which decides what [`RunStore::change`] records on it.
#[derive(Clone, Copy)]
enum Stamp {
    /// The agent: both timestamps, and gate checks in flight are invalidated.
    Agent,
    /// The dashboard: `updated_at` only, and gate checks in flight are invalidated.
    Dashboard,
    /// A gate result was appended: both timestamps; gate checks in flight stay valid, as they tolerate this.
    Gate,
    /// A background collector stored data that gates do not read: no stamp, gate checks stay valid.
    Collector,
}

/// What an edit decided.
enum Edit {
    Save,
    /// Nothing to change: the run is neither stamped nor saved.
    Skip,
}

/// How the gate results may differ from a snapshot when a slow gate check is saved.
enum Gates {
    Unchanged,
    /// Results appended since the snapshot are merged, unless one repeats this result.
    MayAppend(GateResult),
}

/// Whether the stored `current` run changed since `snapshot` in a way that invalidates a gate check.
fn conflicts(current: &Run, snapshot: &Run, gates: &Gates) -> Result<bool, String> {
    if current.gate_generation != snapshot.gate_generation {
        return Ok(true);
    }
    let snapshot_view = gate_view(snapshot)?;
    match gates {
        Gates::Unchanged => Ok(gate_view(current)? != snapshot_view),
        Gates::MayAppend(result) => {
            let mut comparison = current.clone();
            comparison.gates = snapshot.gates.clone();
            Ok(gate_view(&comparison)? != snapshot_view
                || !current.gates.starts_with(&snapshot.gates)
                || current.gates[snapshot.gates.len()..].iter().any(|gate| {
                    gate.gate == result.gate
                        && gate.ticket == result.ticket
                        && gate.check == result.check
                }))
        }
    }
}

impl RunStore {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            shared: Arc::new(Shared {
                path: path.to_owned(),
                runs: Mutex::new(load_runs(path)?),
                collection: Collection::default(),
            }),
        })
    }

    pub(crate) fn usage_collection(&self) -> &Collection {
        &self.shared.collection
    }

    pub(crate) fn list(&self) -> io::Result<Vec<Run>> {
        Ok(self.shared.runs.lock().map_err(lock_error)?.clone())
    }

    pub(crate) fn get(&self, id: &str) -> io::Result<Option<Run>> {
        Ok(self
            .shared
            .runs
            .lock()
            .map_err(lock_error)?
            .iter()
            .find(|run| run.id == id)
            .cloned())
    }

    pub(crate) fn create(&self, new: NewRun) -> io::Result<Run> {
        new.validate()?;
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let mut id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        while runs.iter().any(|run| run.id == id.to_string()) {
            id += 1;
        }
        let created = updated_now()?;
        let run = Run {
            gate_generation: 0,
            gates: Vec::new(),
            check_history: Vec::new(),
            id: id.to_string(),
            title: new.title,
            project: new.project,
            stage: Stage::Define,
            status: RunStatus::Running,
            activity: None,
            current_ticket: None,
            blocking_reason: None,
            rework: None,
            updated_at: Some(created),
            agent_updated_at: Some(created),
            repository: new.repository,
            worktree: None,
            branch: None,
            base_commit: None,
            mode: None,
            spec: None,
            pr: None,
            tickets: Vec::new(),
            ticket_reports: Vec::new(),
            run_reports: Vec::new(),
            findings: Vec::new(),
            summaries: BTreeMap::new(),
            timeline: Vec::new(),
            events: Vec::new(),
            finished_at: None,
            sessions: new.session.into_iter().collect(),
            usage: None,
            pr_status: None,
            model_routing: new.model_routing,
        };
        let mut run = run;
        let created = run.updated_at.unwrap_or(0);
        mark(&mut run, created, None);
        runs.push(run.clone());
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs.pop();
            return Err(error);
        }
        Ok(run)
    }

    /// Locks the store, applies `edit` to a copy of the run, stamps it, saves, and restores the stored
    /// run when saving fails. `edit` gets the copy and the time of the change; an error from it changes
    /// nothing. `Ok(None)` means there is no run with this id.
    fn change(
        &self,
        id: &str,
        stamp: Stamp,
        edit: impl FnOnce(&mut Run, u64) -> io::Result<Edit>,
    ) -> io::Result<Option<Run>> {
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(None);
        };
        let now = updated_now()?;
        let mut next = runs[index].clone();
        if matches!(edit(&mut next, now)?, Edit::Skip) {
            return Ok(Some(runs[index].clone()));
        }
        match stamp {
            Stamp::Agent => {
                next.updated_at = Some(now);
                next.agent_updated_at = Some(now);
                next.gate_generation = next.gate_generation.wrapping_add(1);
            }
            Stamp::Dashboard => {
                next.updated_at = Some(now);
                next.gate_generation = next.gate_generation.wrapping_add(1);
            }
            Stamp::Gate => {
                next.updated_at = Some(now);
                next.agent_updated_at = Some(now);
            }
            Stamp::Collector => {}
        }
        let old = std::mem::replace(&mut runs[index], next);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs[index] = old;
            return Err(error);
        }
        Ok(Some(runs[index].clone()))
    }

    /// For changes that need a slow check first: `check` runs unlocked on a snapshot of the run and returns
    /// what `merge` needs. Once the store is locked again, the run must not have changed since the snapshot
    /// (see [`conflicts`]); then `merge` applies the outcome to the stored run and the change is saved like
    /// [`change`](Self::change).
    fn change_from_snapshot<T>(
        &self,
        id: &str,
        stamp: Stamp,
        check: impl FnOnce(&Run) -> Result<(Gates, T), String>,
        merge: impl FnOnce(&mut Run, T, &Run, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        let not_found = || format!("Run {id} not found");
        let snapshot = self
            .get(id)
            .map_err(|e| format!("Could not read the run: {e}"))?
            .ok_or_else(not_found)?;
        let (gates, value) = check(&snapshot)?;
        let saved = self.change(id, stamp, |current, now| {
            if conflicts(current, &snapshot, &gates).map_err(io_invalid)? {
                return Err(io_invalid(
                    "The run changed while checking gates; try again",
                ));
            }
            merge(current, value, &snapshot, now).map_err(io_invalid)?;
            Ok(Edit::Save)
        });
        match saved {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(not_found()),
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => Err(e.to_string()),
            Err(e) => Err(format!("Could not save the run: {e}")),
        }
    }

    /// Applies an update to a copy first, so an inconsistent update changes nothing, and
    /// restores the stored record when saving fails.
    pub(crate) fn update(&self, id: &str, update: UpdateRun) -> io::Result<Option<Run>> {
        self.update_with(id, update, &crate::run_check::ReadOnlyGit)
    }

    /// Like [`update`](Self::update), reading commits for findings through `git`.
    pub(crate) fn update_with(
        &self,
        id: &str,
        update: UpdateRun,
        git: &dyn crate::pr_sync::Git,
    ) -> io::Result<Option<Run>> {
        update.validate().map_err(io_invalid)?;
        self.change(id, Stamp::Agent, |next, updated_at| {
            let was_blocked = matches!(next.status, RunStatus::Blocked);
            let was_running = matches!(next.status, RunStatus::Running);
            next.status = update.status;
            let started_with = match &update.activity {
                FieldUpdate::Set(text) => Some(text.clone()),
                _ => None,
            };
            update.activity.apply(&mut next.activity);
            update.current_ticket.apply(&mut next.current_ticket);
            update.blocking_reason.apply(&mut next.blocking_reason);
            let stage_before = next.stage;
            let rework_set = matches!(update.rework, FieldUpdate::Set(_));
            update.rework.apply(&mut next.rework);
            update.mode.apply(&mut next.mode);
            update.worktree.apply(&mut next.worktree);
            update.branch.apply(&mut next.branch);
            update.base_commit.map_hex().apply(&mut next.base_commit);
            update.spec.apply(&mut next.spec);
            update.pr.apply(&mut next.pr);
            if (was_running || was_blocked)
                && update.status == RunStatus::Running
                && next.stage == Stage::Check
                && rework_set
            {
                update.findings.check_rework(true).map_err(io_invalid)?;
                let mode = next
                    .mode
                    .ok_or_else(|| io_invalid("Record the run's mode before starting a Rework"))?;
                if let Destination::Stage(stage) = stages::rework(mode, next.stage, next.status)
                    .map_err(io_invalid)?
                    .to
                {
                    next.stage = stage;
                }
            }
            let moved = next.stage != stage_before;
            update
                .findings
                .apply(
                    next,
                    &FindingsContext {
                        stage: stage_before,
                        moved,
                        at: updated_at,
                        git,
                    },
                )
                .map_err(io_invalid)?;
            if update.status == RunStatus::Done && next.stage != Stage::Finish {
                return Err(io_invalid(format!(
                    "A run is done only after Finish; it is in {}. Use `jidoka run advance <id>`",
                    next.stage.name()
                )));
            }
            update.tickets.apply(next).map_err(io_invalid)?;
            update.reports.apply(next, updated_at).map_err(io_invalid)?;
            update.routing.apply(next).map_err(io_invalid)?;
            update.summaries.apply(next);
            if update.status == RunStatus::Done {
                summaries::check_done(next).map_err(io_invalid)?;
            }
            for session in update.sessions {
                if !next.sessions.contains(&session) {
                    next.sessions.push(session);
                }
            }
            if next.sessions.len() > KEEP_SESSIONS {
                next.sessions.drain(..next.sessions.len() - KEEP_SESSIONS);
            }
            record_block_change(next, was_blocked, updated_at);
            // Where the work is now, for attributing token usage; and when it ended.
            mark(next, updated_at, started_with);
            next.finished_at = match (next.status, next.finished_at) {
                (RunStatus::Done, None) => Some(updated_at),
                (RunStatus::Done, kept) => kept,
                _ => None,
            };
            Ok(Edit::Save)
        })
    }

    #[cfg(test)]
    pub(crate) fn set_stage(&self, id: &str, stage: Stage) {
        self.change(id, Stamp::Agent, |run, _| {
            run.stage = stage;
            Ok(Edit::Save)
        })
        .unwrap()
        .expect("test run exists");
    }

    /// Record a gate against a snapshot, merging independent concurrent gate results.
    pub(crate) fn record_gate(
        &self,
        id: &str,
        record: crate::run_gate::Record,
        git: &dyn crate::pr_sync::Git,
    ) -> Result<(), String> {
        self.change_from_snapshot(
            id,
            Stamp::Gate,
            |snapshot| {
                let mut checked = snapshot.clone();
                crate::run_gate::record(&mut checked, record, git)?;
                let result = checked.gates.pop().ok_or("Gate result was not recorded")?;
                Ok((Gates::MayAppend(result.clone()), result))
            },
            |current, result, _, now| {
                if current.gates.len() >= crate::run_gate::MAX_GATE_RESULTS {
                    return Err(format!(
                        "A run can hold at most {} gate results",
                        crate::run_gate::MAX_GATE_RESULTS
                    ));
                }
                current.gates.push(result);
                mark(current, now, None);
                Ok(())
            },
        )
    }

    /// Atomically change a run and persist it, restoring the old value on save failure.
    pub(crate) fn change_gate(
        &self,
        id: &str,
        change: impl FnOnce(&mut Run) -> Result<(), String>,
    ) -> Result<(), String> {
        self.change_from_snapshot(
            id,
            Stamp::Agent,
            |snapshot| {
                let mut changed = snapshot.clone();
                change(&mut changed)?;
                Ok((Gates::Unchanged, changed))
            },
            |current, mut changed, snapshot, now| {
                // Keep what the collectors stored meanwhile (the fields `gate_view` blanks); the change only appends reports.
                let mut reports = current.run_reports.clone();
                reports.extend(
                    changed
                        .run_reports
                        .iter()
                        .skip(snapshot.run_reports.len())
                        .cloned(),
                );
                if reports.len() > MAX_REPORTS_PER_RUN {
                    return Err(format!(
                        "A run can hold at most {MAX_REPORTS_PER_RUN} run reports"
                    ));
                }
                changed.run_reports = reports;
                changed.pr_status = current.pr_status.clone();
                changed.usage = current.usage.clone();
                *current = changed;
                mark(current, now, None);
                if current.status == RunStatus::Done {
                    current.finished_at = Some(now);
                }
                Ok(())
            },
        )
    }

    /// Brings the run in line with the latest pause its log shows (`None` when it shows none). A running
    /// run is blocked for an unresumed pause; a run this method blocked takes the reason of the current
    /// pause, and is set running again once the log shows later activity or no pause at all. Only a state
    /// the dashboard itself set is changed: never a block or reason the agent set, a done run, or a run the
    /// agent updated after the pause began.
    pub(crate) fn apply_pause(&self, id: &str, latest: Option<&Pause>) -> io::Result<()> {
        self.change(id, Stamp::Dashboard, |next, now| {
            let agent_at = next.agent_updated_at.or(next.updated_at);
            let automatic = is_automatic_reason(next.status, next.blocking_reason.as_deref());
            let (kind, at) = match (next.status, latest) {
                (
                    RunStatus::Running,
                    Some(
                        pause @ Pause {
                            resumed_at: None, ..
                        },
                    ),
                ) if Some(pause.at) >= agent_at => {
                    next.status = RunStatus::Blocked;
                    next.blocking_reason = Some(pause.kind.reason().to_string());
                    (RunEventKind::Blocked, pause.at)
                }
                (
                    RunStatus::Blocked,
                    Some(
                        pause @ Pause {
                            resumed_at: None, ..
                        },
                    ),
                ) if automatic && next.blocking_reason.as_deref() != Some(pause.kind.reason()) => {
                    next.blocking_reason = Some(pause.kind.reason().to_string());
                    (RunEventKind::Blocked, pause.at)
                }
                (
                    RunStatus::Blocked,
                    Some(Pause {
                        resumed_at: Some(resumed),
                        ..
                    }),
                ) if automatic => {
                    next.status = RunStatus::Running;
                    next.blocking_reason = None;
                    (RunEventKind::Resumed, *resumed)
                }
                (RunStatus::Blocked, None) if automatic => {
                    next.status = RunStatus::Running;
                    next.blocking_reason = None;
                    (RunEventKind::Resumed, now)
                }
                _ => return Ok(Edit::Skip),
            };
            if next.events.len() < MAX_EVENTS {
                next.events.push(RunEvent {
                    at,
                    kind,
                    stage: next.stage,
                    reason: next.blocking_reason.clone(),
                });
            }
            next.agent_updated_at = agent_at;
            Ok(Edit::Save)
        })
        .map(|_| ())
    }

    /// Replaces the usage of a run. A failed save leaves the stored usage as it was.
    pub(crate) fn set_usage(&self, id: &str, usage: Usage) -> io::Result<()> {
        self.change(id, Stamp::Collector, |run, _| {
            if let Some(routing) = &run.model_routing {
                for row in &usage.rows {
                    let completed = run.status == RunStatus::Done
                        || run.timeline.windows(2).any(|marks| {
                            marks[0].stage == row.stage && marks[1].stage != row.stage
                        });
                    // The coordinator keeps its own model; only subagent usage is judged against the routing.
                    if let Some(choice) = routing.choice_for(row.stage)
                        && completed
                        && !row.subagents.is_empty()
                        && !crate::routing::model_matches(&choice.model, &row.model)
                    {
                        // Log model names are untrusted and can exceed a run report's size limit.
                        let model: String = row.model.chars().take(50).collect();
                        let model = crate::inspect::shown(&model);
                        let text = format!(
                            "{}{} subagent usage recorded model {} while {} was assigned",
                            crate::routing::WARNING_PREFIX,
                            row.stage.name(),
                            model,
                            choice.model
                        );
                        if run.run_reports.len() < MAX_REPORTS_PER_RUN
                            && !run
                                .run_reports
                                .iter()
                                .any(|report| report.stage == row.stage && report.text == text)
                        {
                            run.run_reports.push(RunReport {
                                kind: ReportKind::Activity,
                                text,
                                stage: row.stage,
                                at: usage.collected_at,
                            });
                        }
                    }
                }
            }
            run.usage = Some(usage);
            Ok(Edit::Save)
        })
        .map(|_| ())
    }

    /// Stores the state of a Run's pull request, only while the Run still records that pull request: an update
    /// that changed it in the meantime wins. A failed save leaves the stored state as it was.
    pub(crate) fn set_pr_status(&self, id: &str, status: PrStatus) -> io::Result<()> {
        self.change(id, Stamp::Collector, |run, _| {
            if run.pr.as_deref() != Some(status.url.as_str()) {
                return Ok(Edit::Skip);
            }
            run.pr_status = Some(status);
            Ok(Edit::Save)
        })
        .map(|_| ())
    }

    /// Removes only the Run record. Returns whether a Run with this id existed.
    pub(crate) fn delete(&self, id: &str) -> io::Result<bool> {
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(false);
        };
        let removed = runs.remove(index);
        if let Err(error) = save_runs(&self.shared.path, &runs) {
            runs.insert(index, removed);
            return Err(error);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_state::PrState;
    use crate::stages::Gate;
    use crate::test_support::TestDir;

    fn store_with_run(dir: &TestDir) -> (RunStore, String) {
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = store
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        (store, run.id)
    }

    fn gate(gate: Gate, ticket: &str) -> GateResult {
        GateResult {
            gate,
            stage: Stage::Build,
            rework: None,
            passed: true,
            evidence: None,
            commit: None,
            ticket: Some(ticket.into()),
            check: None,
            skipped_checks: Vec::new(),
        }
    }

    fn append(store: &RunStore, id: &str, result: GateResult) {
        store
            .change(id, Stamp::Gate, |run, _| {
                run.gates.push(result);
                Ok(Edit::Save)
            })
            .unwrap();
    }

    /// Records `result` through a slow check during which `meanwhile` changes the store.
    fn record_during(
        store: &RunStore,
        id: &str,
        result: GateResult,
        meanwhile: impl FnOnce(),
    ) -> Result<(), String> {
        store.change_from_snapshot(
            id,
            Stamp::Gate,
            |_| {
                meanwhile();
                Ok((Gates::MayAppend(result.clone()), result))
            },
            |current, result, _, _| {
                current.gates.push(result);
                Ok(())
            },
        )
    }

    #[test]
    fn a_failed_save_restores_the_stored_run() {
        let dir = TestDir::new();
        let (store, id) = store_with_run(&dir);
        let before = store.get(&id).unwrap().unwrap();
        std::fs::create_dir(dir.0.path().join("runs.json.tmp")).unwrap();
        let result = store.change(&id, Stamp::Agent, |run, _| {
            run.title = "changed".into();
            Ok(Edit::Save)
        });
        assert!(result.is_err());
        let after = store.get(&id).unwrap().unwrap();
        assert_eq!(after.title, before.title);
        assert_eq!(after.gate_generation, before.gate_generation);
    }

    #[test]
    fn a_gate_check_conflicts_with_a_changed_generation_or_gate_view() {
        let dir = TestDir::new();
        let (store, id) = store_with_run(&dir);
        let conflict = "The run changed while checking gates; try again";

        let generation = store.change_from_snapshot(
            &id,
            Stamp::Agent,
            |_| {
                store.set_stage(&id, Stage::Build);
                Ok((Gates::Unchanged, ()))
            },
            |_, (), _, _| Ok(()),
        );
        assert_eq!(generation, Err(conflict.into()));

        let view = store.change_from_snapshot(
            &id,
            Stamp::Collector,
            |_| {
                // A change a gate reads, without the generation bump.
                store
                    .change(&id, Stamp::Collector, |run, _| {
                        run.title = "renamed".into();
                        Ok(Edit::Save)
                    })
                    .unwrap();
                Ok((Gates::Unchanged, ()))
            },
            |_, (), _, _| Ok(()),
        );
        assert_eq!(view, Err(conflict.into()));
    }

    #[test]
    fn collector_fields_stored_during_a_check_are_kept() {
        let dir = TestDir::new();
        let (store, id) = store_with_run(&dir);
        store
            .update(
                &id,
                parse_update("running", &["--pr", "https://example.test/pr/1"]).unwrap(),
            )
            .unwrap();
        store
            .change_gate(&id, |run| {
                run.title = "gate change".into();
                store
                    .set_pr_status(
                        &id,
                        PrStatus {
                            url: "https://example.test/pr/1".into(),
                            state: PrState::Open,
                            reason: None,
                            checked_at: 7,
                        },
                    )
                    .unwrap();
                Ok(())
            })
            .unwrap();
        let run = store.get(&id).unwrap().unwrap();
        assert_eq!(run.title, "gate change");
        assert_eq!(run.pr_status.unwrap().checked_at, 7);
    }

    #[test]
    fn a_gate_result_appended_meanwhile_merges_unless_it_repeats_the_result() {
        let dir = TestDir::new();
        let (store, id) = store_with_run(&dir);
        let other = gate(Gate::Verified, "1");
        record_during(&store, &id, gate(Gate::TicketsDone, "1"), || {
            append(&store, &id, other.clone());
        })
        .unwrap();
        let gates = store.get(&id).unwrap().unwrap().gates;
        assert_eq!(gates.len(), 2);
        assert_eq!(gates[0], other);

        let repeated = record_during(&store, &id, gate(Gate::Verified, "2"), || {
            append(&store, &id, gate(Gate::Verified, "2"));
        });
        assert_eq!(
            repeated,
            Err("The run changed while checking gates; try again".into())
        );
        assert_eq!(store.get(&id).unwrap().unwrap().gates.len(), 3);
    }
}
