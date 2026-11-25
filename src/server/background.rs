//! The work the dashboard server does off the request path: usage collection after an update, the periodic
//! passes that read pull request states and look for pauses, and a pull request read with a time limit.

use crate::{
    pr::{host::PrHost, state as pr_state},
    run::{PrResult, RunStore},
    usage::collection::{UsageSettings, collect_and_store},
};
use std::sync::{Arc, Mutex};

/// Collects token usage in the background after an update that moves a run on. Without settings
/// (as in most tests) nothing is collected.
pub(crate) struct Collector {
    settings: Option<UsageSettings>,
    pending: Mutex<Vec<rocket::tokio::task::JoinHandle<()>>>,
    /// The latest collection chain per run: a newer one replaces an older one's later passes, so a flurry
    /// of updates does not pile up scans.
    chains: Arc<Mutex<std::collections::HashMap<String, u64>>>,
    /// How many passes ran, for tests.
    #[cfg(test)]
    passes: Arc<std::sync::atomic::AtomicUsize>,
}

impl Collector {
    pub(crate) fn new(settings: Option<UsageSettings>) -> Self {
        Self {
            settings,
            pending: Mutex::new(Vec::new()),
            chains: Arc::default(),
            #[cfg(test)]
            passes: Arc::default(),
        }
    }

    /// Starts background collection after a successful update requires it.
    pub(super) fn after_update(&self, store: &RunStore, id: &str) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let (store, id) = (store.clone(), id.to_owned());
        let chains = self.chains.clone();
        let generation = chains.lock().map_or(0, |mut chains| {
            let latest = chains.entry(id.clone()).or_insert(0);
            *latest += 1;
            *latest
        });
        #[cfg(test)]
        let passes = self.passes.clone();
        let handle = rocket::tokio::spawn(async move {
            // One pass now, and one after each delay: the agents' logs lag behind the work.
            let delays = std::iter::once(std::time::Duration::ZERO).chain(settings.settle.clone());
            for delay in delays {
                if !delay.is_zero() {
                    rocket::tokio::time::sleep(delay).await;
                }
                // A newer update started its own chain, which does these passes again.
                if chains
                    .lock()
                    .is_ok_and(|chains| chains.get(&id) != Some(&generation))
                {
                    return;
                }
                let (store, id, settings) = (store.clone(), id.clone(), settings.clone());
                #[cfg(test)]
                passes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let joined = rocket::tokio::task::spawn_blocking({
                    let id = id.clone();
                    move || {
                        if let Err(error) = collect_and_store(&store, &id, &settings) {
                            eprintln!("Usage was not collected for run {id}: {error}");
                        }
                    }
                })
                .await;
                if let Err(error) = joined {
                    eprintln!(
                        "Usage was not collected for run {id}: the collection stopped: {error}"
                    );
                }
            }
            if let Ok(mut chains) = chains.lock()
                && chains.get(&id) == Some(&generation)
            {
                chains.remove(&id);
            }
        });
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|task| !task.is_finished());
            pending.push(handle);
        }
    }

    /// Waits for the collections started so far, for tests.
    #[cfg(test)]
    pub(crate) async fn wait_idle(&self) {
        let handles: Vec<_> = self
            .pending
            .lock()
            .map(|mut p| p.drain(..).collect())
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.await;
        }
    }
}

/// Starts the background passes once the dashboard is serving: one collection of the finished runs whose usage
/// was missed, then the pause and pull request state passes.
pub(super) fn start(
    store: &RunStore,
    tool: Arc<dyn PrHost + Send + Sync>,
    settings: UsageSettings,
) {
    collect_missed_once(store.clone(), settings.clone());
    watch_pauses(store.clone(), settings, PAUSE_PASS);
    watch_pr_states(store.clone(), tool, PR_STATE_PASS);
}

/// How long a sync may wait for the host before the dashboard gives up on it.
const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Why a sync of a Run's pull request gave no answer.
pub(super) enum SyncFailure {
    /// The run storage or the read failed; the reason went to standard error.
    Failed,
    /// The host did not answer within `SYNC_TIMEOUT`.
    TimedOut,
}

/// Reads the state of the Run's pull request from its host now, on a blocking thread, and stores it; gives up after
/// `SYNC_TIMEOUT`. `None` when the Run has no pull request to read.
pub(super) async fn sync_pr_now(
    store: RunStore,
    tool: Arc<dyn PrHost + Send + Sync>,
    id: String,
) -> Result<Option<(pr_state::PrStatus, PrResult)>, SyncFailure> {
    let read = rocket::tokio::task::spawn_blocking(move || {
        let now = crate::run::updated_now()?;
        pr_state::sync_run(&store, tool.as_ref(), &id, now)
    });
    match rocket::tokio::time::timeout(SYNC_TIMEOUT, read).await {
        Ok(Ok(Ok(synced))) => Ok(synced),
        Ok(Ok(Err(error))) => {
            eprintln!("Run storage error: {error}");
            Err(SyncFailure::Failed)
        }
        Ok(Err(error)) => {
            eprintln!("Pull request sync stopped: {error}");
            Err(SyncFailure::Failed)
        }
        Err(_) => Err(SyncFailure::TimedOut),
    }
}

/// How often the dashboard server looks for pull requests whose state is due to be read again. Every 20 seconds
/// keeps the dashboard fresh while each pass only reads the pull requests that are due, so hosts are not hammered.
const PR_STATE_PASS: std::time::Duration = std::time::Duration::from_secs(20);

/// Reads the state of the Runs' pull requests from their hosts, one pass every `pass`, on a blocking thread so
/// a slow or missing tool never holds up a request. Merged pull requests are not read again.
fn watch_pr_states(
    store: RunStore,
    tool: Arc<dyn PrHost + Send + Sync>,
    pass: std::time::Duration,
) -> rocket::tokio::task::JoinHandle<()> {
    rocket::tokio::spawn(async move {
        loop {
            let (store, tool) = (store.clone(), tool.clone());
            let joined = rocket::tokio::task::spawn_blocking(move || {
                if let Err(error) = pr_state::refresh_live(&store, tool.as_ref()) {
                    eprintln!("Pull request states were not stored: {error}");
                }
            })
            .await;
            if let Err(error) = joined {
                eprintln!("Pull request states were not read: the read stopped: {error}");
            }
            rocket::tokio::time::sleep(pass).await;
        }
    })
}

/// How often the dashboard server looks in the logs of working runs for a usage limit or compaction. A minute is
/// frequent enough to notice a pause promptly and rare enough that reading the logs stays cheap.
const PAUSE_PASS: std::time::Duration = std::time::Duration::from_secs(60);

/// Collects the runs that are working or paused on their own every `pass`, on a blocking thread, so a
/// pause is seen and cleared without an agent update. One pass is bounded by the collection budget per run.
fn watch_pauses(
    store: RunStore,
    settings: UsageSettings,
    pass: std::time::Duration,
) -> rocket::tokio::task::JoinHandle<()> {
    rocket::tokio::spawn(async move {
        loop {
            rocket::tokio::time::sleep(pass).await;
            let (store, settings) = (store.clone(), settings.clone());
            let joined = rocket::tokio::task::spawn_blocking(move || {
                let result = crate::run::updated_now().and_then(|now| {
                    crate::usage::collection::collect_active(&store, &settings, now)
                });
                if let Err(error) = result {
                    eprintln!("Pauses were not checked: {error}");
                }
            })
            .await;
            if let Err(error) = joined {
                eprintln!("Pauses were not checked: the check stopped: {error}");
            }
        }
    })
}

/// Collects, once at startup and on a blocking thread, the recent done runs whose usage was not tracked, so
/// a dashboard started in the agents' environment recovers runs an earlier dashboard could not read.
fn collect_missed_once(store: RunStore, settings: UsageSettings) {
    rocket::tokio::spawn(async move {
        let joined = rocket::tokio::task::spawn_blocking(move || {
            let result = crate::run::updated_now()
                .and_then(|now| crate::usage::collection::collect_missed(&store, &settings, now));
            if let Err(error) = result {
                eprintln!("Usage of finished runs was not collected again: {error}");
            }
        })
        .await;
        if let Err(error) = joined {
            eprintln!(
                "Usage of finished runs was not collected again: the collection stopped: {error}"
            );
        }
    });
}
