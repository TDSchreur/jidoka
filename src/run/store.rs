//! The run records and the one way to change them safely.

use super::*;

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
/// The collector-owned fields, which gates never read and background collectors write meanwhile, are listed
/// only here and blanked, so they cannot make a slow gate check fail spuriously: the Pull request status,
/// Token usage, run reports and the timestamps. A slow change applies its outcome to the stored run, so the
/// values collected meanwhile are kept.
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

enum UpdateSource {
    Report,
    WorkspaceReady,
}

/// What an edit decided.
enum Edit {
    Save,
    /// A collector's edit that changed the lifecycle at `at`, read by the collector.
    ///
    /// It is stamped like a dashboard change (`updated_at`, gate checks in flight invalidated) and marked
    /// on the Timeline, not as agent activity.
    SaveLifecycle {
        at: u64,
    },
    /// Nothing to change: the run is neither stamped nor saved.
    Skip,
}

/// What an advance records on the run along with its Stage exit.
#[derive(Clone, Debug, Default)]
pub(crate) struct AdvanceMetadata {
    pub(crate) activity: Option<String>,
    pub(crate) summaries: SummariesUpdate,
    pub(crate) sessions: Vec<String>,
    pub(crate) agent_homes: BTreeMap<String, String>,
}

impl AdvanceMetadata {
    fn apply(self, run: &mut Run) {
        if let Some(activity) = self.activity {
            run.activity = Some(activity);
        }
        self.summaries.apply(run);
        run.record_sessions(self.sessions);
        run.record_agent_homes(self.agent_homes);
    }
}

/// How the gate results may differ from a snapshot when a slow gate check is saved.
enum Gates {
    Unchanged,
    /// Results appended since the snapshot are merged, unless one repeats this result.
    MayAppend(GateResult),
}

const CHANGED_WHILE_CHECKING: &str = "The run changed while checking gates; try again";

/// Why [`RunStore::change_with`] failed, so callers can word each case their own way.
enum ChangeError<E> {
    Edit(E),
    Lock(String),
    Clock(io::Error),
    Save(io::Error),
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
        let id = new_id(|candidate| runs.iter().any(|run| run.id == candidate))?;
        let created = updated_now()?;
        let run = Run {
            gate_generation: 0,
            gates: Vec::new(),
            stage_exits: Vec::new(),
            id,
            title: new.title,
            project: new.project,
            stage: Stage::Define,
            status: RunStatus::Running,
            activity: None,
            blocking_reason: None,
            block_source: None,
            rework: None,
            rework_history: Vec::new(),
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
            unticketed_attempt: None,
            last_work_attempt: 0,
            ticket_reports: Vec::new(),
            run_reports: Vec::new(),
            findings: Vec::new(),
            summaries: BTreeMap::new(),
            timeline: Vec::new(),
            events: Vec::new(),
            finished_at: None,
            completion: None,
            sessions: new.session.into_iter().collect(),
            agent_homes: new.agent_homes,
            usage: None,
            pr_status: None,
            model_routing: new.model_routing,
        };
        let mut run = run;
        let created = run.updated_at.unwrap_or(0);
        mark(&mut run, created, None);
        runs.push(run.clone());
        if let Err(error) = save_records(&self.shared.path, &runs) {
            runs.pop();
            return Err(error);
        }
        Ok(run)
    }

    /// Locks the store, applies `edit` to a copy of the run, stamps it and saves it.
    ///
    /// `edit` gets the copy and the time of the change, read once for the whole change; collector changes
    /// go through [`collect`](Self::collect) and read no time. An error from `edit` changes nothing, and a
    /// failed save restores the stored run. `Ok(None)` means there is no run with this id.
    fn change(
        &self,
        id: &str,
        stamp: Stamp,
        edit: impl FnOnce(&mut Run, u64) -> io::Result<Edit>,
    ) -> io::Result<Option<Run>> {
        self.change_with(id, stamp, edit)
            .map_err(|error| match error {
                ChangeError::Edit(e) | ChangeError::Clock(e) | ChangeError::Save(e) => e,
                ChangeError::Lock(_) => io::Error::other("run storage lock was poisoned"),
            })
    }

    fn change_with<E>(
        &self,
        id: &str,
        stamp: Stamp,
        edit: impl FnOnce(&mut Run, u64) -> Result<Edit, E>,
    ) -> Result<Option<Run>, ChangeError<E>> {
        let mut runs = self
            .shared
            .runs
            .lock()
            .map_err(|e| ChangeError::Lock(e.to_string()))?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(None);
        };
        let now = match stamp {
            // Collector edits come through `collect`, which hands them no time, so the clock is not read.
            Stamp::Collector => 0,
            _ => updated_now().map_err(ChangeError::Clock)?,
        };
        let mut next = runs[index].clone();
        let (stamp, now) = match edit(&mut next, now).map_err(ChangeError::Edit)? {
            Edit::Save => (stamp, now),
            Edit::SaveLifecycle { at } => {
                mark(&mut next, at, None);
                (Stamp::Dashboard, at)
            }
            Edit::Skip => return Ok(Some(runs[index].clone())),
        };
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
        if let Err(error) = save_records(&self.shared.path, &runs) {
            runs[index] = old;
            return Err(ChangeError::Save(error));
        }
        Ok(Some(runs[index].clone()))
    }

    /// Stores what a background collector gathered: no stamp, and gate checks in flight stay valid.
    ///
    /// The edit gets no time; one that changes the lifecycle reads it and returns [`Edit::SaveLifecycle`].
    fn collect(
        &self,
        id: &str,
        edit: impl FnOnce(&mut Run) -> io::Result<Edit>,
    ) -> io::Result<Option<Run>> {
        self.change(id, Stamp::Collector, |run, _| edit(run))
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
        merge: impl FnOnce(&mut Run, T, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        let not_found = || format!("Run {id} not found");
        let snapshot = self
            .get(id)
            .map_err(|e| format!("Could not read the run: {e}"))?
            .ok_or_else(not_found)?;
        let (gates, value) = check(&snapshot)?;
        let saved = self.change_with(id, stamp, |current, now| {
            if conflicts(current, &snapshot, &gates)? {
                return Err(CHANGED_WHILE_CHECKING.to_string());
            }
            merge(current, value, now)?;
            Ok(Edit::Save)
        });
        match saved {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(not_found()),
            Err(ChangeError::Edit(message) | ChangeError::Lock(message)) => Err(message),
            Err(ChangeError::Clock(e)) => Err(format!("Could not read the time: {e}")),
            Err(ChangeError::Save(e)) => Err(format!("Could not save the run: {e}")),
        }
    }

    /// Applies an update to a copy first, so an inconsistent update changes nothing, and
    /// restores the stored record when saving fails.
    pub(crate) fn update(&self, id: &str, update: UpdateRun) -> io::Result<Option<Run>> {
        self.update_with(id, update, &crate::git::ReadOnlyGit)
    }

    /// Like [`update`](Self::update), reading commits for findings through `git`.
    pub(crate) fn update_with(
        &self,
        id: &str,
        update: UpdateRun,
        git: &dyn crate::git::Git,
    ) -> io::Result<Option<Run>> {
        self.apply_update(id, update, git, UpdateSource::Report)
    }

    fn apply_update(
        &self,
        id: &str,
        update: UpdateRun,
        git: &dyn crate::git::Git,
        source: UpdateSource,
    ) -> io::Result<Option<Run>> {
        update.validate().map_err(io_invalid)?;
        self.change(id, Stamp::Agent, |next, updated_at| {
            if matches!(source, UpdateSource::WorkspaceReady) {
                if next.status == RunStatus::Done || next.worktree.is_some() {
                    return Err(io_invalid(
                        "Only an unfinished run without a worktree can record a new workspace",
                    ));
                }
                if !matches!(update.worktree, FieldUpdate::Set(_)) {
                    return Err(io_invalid("Workspace ready requires a worktree"));
                }
            }
            if let FieldUpdate::Set(mode) = update.mode {
                if next.mode.is_some_and(|stored| stored != mode) {
                    return Err(io_invalid("The workflow mode is already initialized"));
                }
                if next.mode.is_none() {
                    let stage = next.stage;
                    reports::observe_skill(next, mode.name(), stage, updated_at);
                    next.mode = Some(mode);
                }
            }
            let started_with = match &update.activity {
                FieldUpdate::Set(text) => Some(text.clone()),
                _ => None,
            };
            update.activity.apply(&mut next.activity);
            update.worktree.apply(&mut next.worktree);
            update.branch.apply(&mut next.branch);
            update.base_commit.map_hex().apply(&mut next.base_commit);
            update.spec.apply(&mut next.spec);
            update.pr.apply(&mut next.pr);
            update
                .findings
                .apply(
                    next,
                    &FindingsContext {
                        stage: next.stage,
                        at: updated_at,
                        git,
                    },
                )
                .map_err(io_invalid)?;
            update.tickets.apply(next).map_err(io_invalid)?;
            update.reports.apply(next, updated_at).map_err(io_invalid)?;
            update.routing.apply(next).map_err(io_invalid)?;
            update.summaries.apply(next);
            if next.completion.is_some() {
                next.check_finish_summary().map_err(io_invalid)?;
            }
            next.record_sessions(update.sessions);
            next.record_agent_homes(update.agent_homes);
            if matches!(source, UpdateSource::WorkspaceReady)
                && next.block_source == Some(LifecycleSource::Workspace)
            {
                next.resume("Workspace ready", LifecycleSource::Workspace, updated_at)
                    .map_err(io_invalid)?;
            }
            // Where the work is now, for attributing token usage.
            mark(next, updated_at, started_with);
            Ok(Edit::Save)
        })
    }

    /// Applies an explicit lifecycle request under the store lock.
    pub(crate) fn transition(
        &self,
        id: &str,
        request: LifecycleRequest,
    ) -> io::Result<Option<Run>> {
        if let LifecycleRequest::WorkspaceReady { update } = request {
            return self.apply_update(
                id,
                *update,
                &crate::git::ReadOnlyGit,
                UpdateSource::WorkspaceReady,
            );
        }
        self.change(id, Stamp::Agent, |run, now| {
            match request {
                LifecycleRequest::Block { reason } => {
                    run.block(&reason, LifecycleSource::Agent, now)
                }
                LifecycleRequest::Resume { source } => {
                    run.resume(&source, LifecycleSource::Agent, now)
                }
                LifecycleRequest::Rework { findings, decision } => {
                    run.leave(Exit::Rework { findings, decision }, now)
                }
                LifecycleRequest::WorkspaceBlocked { reason } => {
                    if run.block_source == Some(LifecycleSource::Workspace) {
                        run.refresh_block(&reason, LifecycleSource::Workspace, now)
                    } else {
                        run.block(&reason, LifecycleSource::Workspace, now)
                    }
                }
                LifecycleRequest::WorkspaceReady { .. } => {
                    Err("Workspace update was already handled".into())
                }
            }
            .map_err(io_invalid)?;
            mark(run, now, None);
            Ok(Edit::Save)
        })
    }

    /// Records the gate result `check` returns for a snapshot, merging independent concurrent gate results.
    ///
    /// # Errors
    ///
    /// Returns a message when the run is missing, `check` refuses, the run changed in a way that
    /// invalidates the result, the run holds too many gate results, or it cannot be saved.
    pub(crate) fn record_gate(
        &self,
        id: &str,
        check: impl FnOnce(&Run) -> Result<GateResult, String>,
    ) -> Result<(), String> {
        self.change_from_snapshot(
            id,
            Stamp::Gate,
            |snapshot| {
                let result = check(snapshot)?;
                Ok((Gates::MayAppend(result.clone()), result))
            },
            |current, result, now| {
                if current.gates.len() >= MAX_GATE_RESULTS {
                    return Err(format!(
                        "A run can hold at most {MAX_GATE_RESULTS} gate results"
                    ));
                }
                current.gates.push(result);
                mark(current, now, None);
                Ok(())
            },
        )
    }

    /// Leaves the run's Stage with the exit `check` decides. `check` runs unlocked on a snapshot with
    /// `metadata` applied (the Finish summary gate reads summaries) and never changes the stored run. Once
    /// the store is locked again and the run has not changed since the snapshot (see [`conflicts`]), the
    /// metadata and the exit are applied to the stored run, both at one clock reading.
    ///
    /// # Errors
    ///
    /// Returns a message when the run is missing, `check` refuses, the run changed while checking, the run
    /// cannot leave its Stage this way, or it cannot be saved.
    pub(crate) fn advance(
        &self,
        id: &str,
        metadata: AdvanceMetadata,
        check: impl FnOnce(&Run) -> Result<Exit, String>,
    ) -> Result<(), String> {
        let activity = metadata.activity.clone();
        let for_view = metadata.clone();
        self.change_from_snapshot(
            id,
            Stamp::Agent,
            |snapshot| {
                let mut view = snapshot.clone();
                for_view.apply(&mut view);
                Ok((Gates::Unchanged, check(&view)?))
            },
            |current, exit, now| {
                metadata.apply(current);
                current.leave(exit, now)?;
                mark(current, now, activity);
                Ok(())
            },
        )
    }

    /// Brings the run in line with the latest pause its log shows (`None` when it shows none). A running
    /// run is blocked for an unresumed pause; a run this method blocked takes the reason of the current
    /// pause, and is set running again once the log shows later activity or no pause at all. Only a state
    /// the dashboard itself set is changed: never a block or reason the agent set, a done run, or a run the
    /// agent updated after the pause began, unless an explicit request remains unanswered.
    pub(crate) fn apply_pause(
        &self,
        id: &str,
        latest: Option<&Pause>,
        requested: bool,
    ) -> io::Result<()> {
        self.change(id, Stamp::Dashboard, |next, now| {
            let agent_at = next.agent_updated_at.or(next.updated_at);
            let automatic = next.automatically_blocked();
            match (next.status, latest) {
                (
                    RunStatus::Running,
                    Some(
                        pause @ Pause {
                            resumed_at: None, ..
                        },
                    ),
                ) if requested || Some(pause.at) >= agent_at => {
                    next.block(pause.kind.reason(), LifecycleSource::Pause, pause.at)
                }
                (
                    RunStatus::Blocked,
                    Some(
                        pause @ Pause {
                            resumed_at: None, ..
                        },
                    ),
                ) if automatic && next.blocking_reason.as_deref() != Some(pause.kind.reason()) => {
                    next.refresh_block(pause.kind.reason(), LifecycleSource::Pause, pause.at)
                }
                (
                    RunStatus::Blocked,
                    Some(Pause {
                        resumed_at: Some(at),
                        ..
                    }),
                ) if automatic => {
                    next.resume("Agent activity resumed", LifecycleSource::Pause, *at)
                }
                (RunStatus::Blocked, None) if automatic => next.resume(
                    "Pause cleared from the agent log",
                    LifecycleSource::Pause,
                    now,
                ),
                _ => return Ok(Edit::Skip),
            }
            .map_err(io_invalid)?;
            next.agent_updated_at = agent_at;
            Ok(Edit::Save)
        })
        .map(|_| ())
    }

    /// Replaces the usage of a run. A failed save leaves the stored usage as it was.
    pub(crate) fn set_usage(&self, id: &str, usage: Usage) -> io::Result<()> {
        self.collect(id, |run| {
            if let Some(routing) = &run.model_routing {
                for row in &usage.rows {
                    let completed = run.status == RunStatus::Done
                        || run.timeline.windows(2).any(|marks| {
                            marks[0].stage == row.stage && marks[1].stage != row.stage
                        });
                    // The coordinator keeps its own model; only subagent usage is judged against the routing.
                    // Without a current route, coordinator and subagent work may both be legitimate.
                    if let Some(choice) = routing.choice_for(row.stage).or_else(|| {
                        (row.stage == Stage::Finish)
                            .then(|| routing.choice_for(Stage::Build))
                            .flatten()
                    }) && completed
                        && !row.subagents.is_empty()
                        && !routing.allows_model(row.stage, &row.model)
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

    /// Records the skills the session logs show used as run skill reports, each once per stage; skills the
    /// agent already reported are not repeated. Saves only when a report was added.
    pub(crate) fn observe_skills(
        &self,
        id: &str,
        skills: &[crate::usage::token_usage::UsedSkill],
    ) -> io::Result<()> {
        if skills.is_empty() {
            return Ok(());
        }
        self.collect(id, |run| {
            let mut added = false;
            for skill in skills {
                added |= reports::observe_skill(run, &skill.name, skill.stage, skill.at);
            }
            Ok(if added { Edit::Save } else { Edit::Skip })
        })
        .map(|_| ())
    }

    /// Stores the state of a Run's pull request, only while the Run still records that pull request: an update
    /// that changed it in the meantime wins. A failed save leaves the stored state as it was.
    #[cfg(test)]
    pub(crate) fn set_pr_status(&self, id: &str, status: PrStatus) -> io::Result<()> {
        self.collect(id, |run| {
            if run.pr.as_deref() != Some(status.url.as_str()) {
                return Ok(Edit::Skip);
            }
            run.pr_status = Some(status);
            Ok(Edit::Save)
        })
        .map(|_| ())
    }

    /// Records a host result and its run transition together, only for the current pull request.
    pub(crate) fn record_pr_result(&self, id: &str, status: PrStatus) -> io::Result<PrResult> {
        let mut result = PrResult::Discarded;
        let found = self.collect(id, |run| {
            if run.pr.as_deref() != Some(status.url.as_str())
                || run
                    .pr_status
                    .as_ref()
                    .is_some_and(|previous| !status.supersedes(previous))
            {
                return Ok(Edit::Skip);
            }
            let now = updated_now()?;
            let changed = crate::pr_state::follow_up(run, &status, now).map_err(io_invalid)?;
            result = PrResult::Accepted { changed };
            run.pr_status = Some(status);
            Ok(if changed {
                Edit::SaveLifecycle { at: now }
            } else {
                Edit::Save
            })
        })?;
        Ok(if found.is_some() {
            result
        } else {
            PrResult::Missing
        })
    }

    /// Removes only the Run record. Returns whether a Run with this id existed.
    pub(crate) fn delete(&self, id: &str) -> io::Result<bool> {
        let mut runs = self.shared.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(false);
        };
        let removed = runs.remove(index);
        if let Err(error) = save_records(&self.shared.path, &runs) {
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
        let run = crate::test_support::new_run(&store, "task");
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
            work_attempt: None,
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
            |current, result, _| {
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
        // A directory at the temp file name `save_records` writes to makes the save fail.
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
        let conflict = CHANGED_WHILE_CHECKING;

        let generation = store.change_from_snapshot(
            &id,
            Stamp::Agent,
            |_| {
                // An agent change bumps the generation, even one that changes nothing a gate reads.
                store
                    .change(&id, Stamp::Agent, |_, _| Ok(Edit::Save))
                    .unwrap();
                Ok((Gates::Unchanged, ()))
            },
            |_, (), _| Ok(()),
        );
        assert_eq!(generation, Err(conflict.into()));

        let view = store.change_from_snapshot(
            &id,
            Stamp::Collector,
            |_| {
                // A change a gate reads, without the generation bump.
                store
                    .collect(&id, |run| {
                        run.title = "renamed".into();
                        Ok(Edit::Save)
                    })
                    .unwrap();
                Ok((Gates::Unchanged, ()))
            },
            |_, (), _| Ok(()),
        );
        assert_eq!(view, Err(conflict.into()));
    }

    #[test]
    fn an_advance_keeps_what_a_collector_stored_during_its_check() {
        let dir = TestDir::new();
        let (store, id) = store_with_run(&dir);
        store
            .update(
                &id,
                parse_update(&["--mode", "fast", "--pr", "https://example.test/pr/1"]).unwrap(),
            )
            .unwrap();
        let usage: Usage =
            serde_json::from_value(serde_json::json!({"status":"tracked","collected_at":7}))
                .unwrap();
        let metadata = AdvanceMetadata {
            activity: Some("Building".into()),
            ..AdvanceMetadata::default()
        };
        store
            .advance(&id, metadata, |snapshot| {
                assert_eq!(snapshot.activity.as_deref(), Some("Building"));
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
                store.set_usage(&id, usage.clone()).unwrap();
                let skill = crate::usage::token_usage::UsedSkill {
                    stage: Stage::Define,
                    name: "grilling".into(),
                    at: 7,
                };
                store.observe_skills(&id, &[skill]).unwrap();
                Ok(Exit::Override {
                    reason: "Spec agreed by hand".into(),
                })
            })
            .unwrap();
        let run = store.get(&id).unwrap().unwrap();
        assert_eq!(run.stage, Stage::Build);
        assert_eq!(run.activity.as_deref(), Some("Building"));
        assert_eq!(run.pr_status.unwrap().checked_at, 7);
        assert_eq!(run.usage, Some(usage));
        // The skill report observed during the check is kept, ahead of the forced advance's report.
        let observed = &run.run_reports[run.run_reports.len() - 2];
        assert!(
            matches!(observed.kind, ReportKind::Skill) && observed.at == 7,
            "the skill report observed during the check should precede the forced advance's report, got {:?} at {}",
            observed.kind,
            observed.at
        );
        assert_eq!(
            run.run_reports.last().unwrap().text,
            "Forced advance from define: Spec agreed by hand"
        );
        assert_eq!(run.stage_exits[0].at, run.updated_at.unwrap());
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrResult {
    Accepted { changed: bool },
    Discarded,
    Missing,
}
