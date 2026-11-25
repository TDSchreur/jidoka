//! Helpers that tests across the crate share.

use crate::run::{AdvanceMetadata, Exit, NewRun, Run, RunStore, Stage, parse_update};

pub(crate) struct TestDir(pub(crate) tempfile::TempDir);
impl TestDir {
    pub(crate) fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
}

/// Creates a run titled `title` in `store`, for a worktree of `/work/repo`.
///
/// # Panics
///
/// Panics when the store cannot create the run, for example because its data folder is not writable.
pub(crate) fn new_run(store: &RunStore, title: &str) -> Run {
    store
        .create(NewRun {
            title: title.into(),
            project: "/work/repo.worktrees/feature".into(),
            repository: Some("/work/repo".into()),
            session: None,
            agent_homes: Default::default(),
            model_routing: None,
        })
        .unwrap()
}

/// Moves a stored run forward to `stage` through `RunStore::advance` with Override exits.
///
/// The run moves one Stage at a time, as in production. A run without a mode gets the balanced mode
/// first, as an Override needs one; record another mode before calling this when the test needs it.
///
/// # Panics
///
/// Panics when the run does not exist or cannot reach `stage`, for example because it is already past it.
pub(crate) fn advance_to(store: &RunStore, id: &str, stage: Stage) {
    let run = store.get(id).unwrap().expect("test run exists");
    if run.mode.is_none() {
        store
            .update(id, parse_update(&["--mode", "balanced"]).unwrap())
            .unwrap();
    }
    while store.get(id).unwrap().unwrap().stage != stage {
        force_advance(store, id, None);
    }
}

/// Leaves the stored run's current Stage with an Override exit, saving `activity` along with it.
///
/// # Panics
///
/// Panics when the run does not exist or cannot leave its Stage, for example because it has no mode.
pub(crate) fn force_advance(store: &RunStore, id: &str, activity: Option<&str>) -> Run {
    let metadata = AdvanceMetadata {
        activity: activity.map(str::to_owned),
        ..AdvanceMetadata::default()
    };
    store
        .advance(id, metadata, |_| {
            Ok(Exit::Override {
                reason: "Test moves the run on".into(),
            })
        })
        .unwrap_or_else(|error| panic!("cannot advance run {id}: {error}"));
    store.get(id).unwrap().expect("test run exists")
}
