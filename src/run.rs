use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const UPDATE_HELP: &str = "Run update options:\n  --activity <text> | --clear-activity\n  --current-ticket <http(s)-url> | --clear-current-ticket\n  --blocking-reason <text> | --clear-blocking-reason\n  --rework <1-99> | --clear-rework\nOmitted fields are preserved; clear flags remove them.";
/// Highest Rework count a run can report.
pub(crate) const MAX_REWORK: u32 = 99;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Run {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) project: String,
    pub(crate) stage: Stage,
    pub(crate) status: RunStatus,
    #[serde(default)]
    pub(crate) activity: Option<String>,
    #[serde(default)]
    pub(crate) current_ticket: Option<String>,
    #[serde(default)]
    pub(crate) blocking_reason: Option<String>,
    /// Number of the current or latest Rework: a return from Check to Build.
    #[serde(default)]
    pub(crate) rework: Option<u32>,
    #[serde(default)]
    pub(crate) updated_at: Option<u64>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Stage {
    Define,
    Build,
    Check,
    Finish,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RunStatus {
    Running,
    Blocked,
    Done,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct NewRun {
    pub(crate) title: String,
    pub(crate) project: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateRun {
    pub(crate) stage: Stage,
    pub(crate) status: RunStatus,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) activity: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) current_ticket: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) blocking_reason: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    pub(crate) rework: FieldUpdate<u32>,
}

// Missing fields preserve stored values; null clears them; values replace them.
#[derive(Default, Serialize)]
#[serde(untagged)]
pub(crate) enum FieldUpdate<T = String> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FieldUpdate<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Set(value),
            None => Self::Clear,
        })
    }
}

impl<T> FieldUpdate<T> {
    pub(crate) fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    pub(crate) fn apply(self, target: &mut Option<T>) {
        match self {
            Self::Keep => {}
            Self::Clear => *target = None,
            Self::Set(value) => *target = Some(value),
        }
    }
}

impl FieldUpdate {
    pub(crate) fn validate(&self, limit: usize) -> Result<(), &'static str> {
        if let Self::Set(value) = self {
            if value.trim().is_empty() || value.len() > limit {
                return Err("Run metadata must be nonblank and within its size limit");
            }
        }
        Ok(())
    }
}

impl UpdateRun {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.activity.validate(2000)?;
        self.blocking_reason.validate(2000)?;
        self.current_ticket.validate(4096)?;
        if let FieldUpdate::Set(count) = self.rework {
            if !(1..=MAX_REWORK).contains(&count) {
                return Err("Rework must be a whole number from 1 to 99");
            }
        }
        if let FieldUpdate::Set(value) = &self.current_ticket {
            let valid = rocket::http::uri::Absolute::parse(value)
                .ok()
                .is_some_and(|uri| {
                    matches!(uri.scheme(), "http" | "https")
                        && uri
                            .authority()
                            .is_some_and(|authority| !authority.host().is_empty())
                });
            if !valid {
                return Err("Current ticket must be an absolute HTTP(S) URL");
            }
        }
        Ok(())
    }
}

pub(crate) fn parse_update(
    stage: &str,
    status: &str,
    options: &[&str],
) -> Result<UpdateRun, Box<dyn std::error::Error>> {
    let mut update = UpdateRun {
        stage: serde_json::from_value(stage.into())?,
        status: serde_json::from_value(status.into())?,
        activity: FieldUpdate::Keep,
        current_ticket: FieldUpdate::Keep,
        blocking_reason: FieldUpdate::Keep,
        rework: FieldUpdate::Keep,
    };
    let mut args = options.iter();
    while let Some(flag) = args.next() {
        let mut next_value = || {
            args.next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("Missing value for {flag}\n{UPDATE_HELP}"))
        };
        if matches!(*flag, "--rework" | "--clear-rework") {
            if !update.rework.is_keep() {
                return Err(format!("Repeated or conflicting run update option: {flag}").into());
            }
            update.rework = if *flag == "--clear-rework" {
                FieldUpdate::Clear
            } else {
                let count = next_value()?;
                FieldUpdate::Set(count.parse().map_err(|_| {
                    format!("Invalid value for --rework: {count}; use a whole number from 1 to {MAX_REWORK}")
                })?)
            };
            continue;
        }
        let field = match *flag {
            "--activity" | "--clear-activity" => &mut update.activity,
            "--current-ticket" | "--clear-current-ticket" => &mut update.current_ticket,
            "--blocking-reason" | "--clear-blocking-reason" => &mut update.blocking_reason,
            _ => return Err(format!("Unknown run update option: {flag}\n{UPDATE_HELP}").into()),
        };
        if !field.is_keep() {
            return Err(format!("Repeated or conflicting run update option: {flag}").into());
        }
        *field = if flag.starts_with("--clear-") {
            FieldUpdate::Clear
        } else {
            FieldUpdate::Set((*next_value()?).to_string())
        };
    }
    update.validate()?;
    Ok(update)
}

pub(crate) fn updated_now() -> io::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    u64::try_from(millis).map_err(io::Error::other)
}

pub(crate) struct RunStore {
    path: PathBuf,
    runs: Mutex<Vec<Run>>,
}

impl RunStore {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: path.to_owned(),
            runs: Mutex::new(load_runs(path)?),
        })
    }

    pub(crate) fn list(&self) -> io::Result<Vec<Run>> {
        Ok(self.runs.lock().map_err(lock_error)?.clone())
    }

    pub(crate) fn create(&self, new: NewRun) -> io::Result<Run> {
        let mut runs = self.runs.lock().map_err(lock_error)?;
        let mut id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        while runs.iter().any(|run| run.id == id.to_string()) {
            id += 1;
        }
        let run = Run {
            id: id.to_string(),
            title: new.title,
            project: new.project,
            stage: Stage::Define,
            status: RunStatus::Running,
            activity: None,
            current_ticket: None,
            blocking_reason: None,
            rework: None,
            updated_at: Some(updated_now()?),
        };
        runs.push(run.clone());
        if let Err(error) = save_runs(&self.path, &runs) {
            runs.pop();
            return Err(error);
        }
        Ok(run)
    }

    pub(crate) fn update(&self, id: &str, update: UpdateRun) -> io::Result<Option<Run>> {
        let mut runs = self.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(None);
        };
        let updated_at = updated_now()?;
        let old = runs[index].clone();
        runs[index].stage = update.stage;
        runs[index].status = update.status;
        update.activity.apply(&mut runs[index].activity);
        update.current_ticket.apply(&mut runs[index].current_ticket);
        update
            .blocking_reason
            .apply(&mut runs[index].blocking_reason);
        update.rework.apply(&mut runs[index].rework);
        runs[index].updated_at = Some(updated_at);
        if let Err(error) = save_runs(&self.path, &runs) {
            runs[index] = old;
            return Err(error);
        }
        Ok(Some(runs[index].clone()))
    }
}

pub(crate) fn lock_error<T>(_: std::sync::PoisonError<T>) -> io::Error {
    io::Error::other("run storage lock was poisoned")
}

pub(crate) fn load_runs(path: &Path) -> io::Result<Vec<Run>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

pub(crate) fn save_runs(path: &Path, runs: &[Run]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("run storage has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec(runs)?)?;
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn concurrent_creates_keep_all_records_and_unique_ids() {
        let dir = TestDir::new();
        let store = std::sync::Arc::new(RunStore::open(&dir.0.path().join("runs.json")).unwrap());
        let threads: Vec<_> = (0..16)
            .map(|n| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .create(NewRun {
                            title: format!("run {n}"),
                            project: "/work".into(),
                        })
                        .unwrap()
                })
            })
            .collect();
        let created: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let ids: std::collections::HashSet<_> = created.iter().map(|run| run.id.clone()).collect();
        assert_eq!(ids.len(), 16);
        assert_eq!(
            load_runs(&dir.0.path().join("runs.json")).unwrap().len(),
            16
        );
    }

    #[test]
    fn concurrent_updates_to_different_runs_are_both_persisted() {
        let dir = TestDir::new();
        let store = std::sync::Arc::new(RunStore::open(&dir.0.path().join("runs.json")).unwrap());
        let first = store
            .create(NewRun {
                title: "first".into(),
                project: "/work".into(),
            })
            .unwrap();
        let second = store
            .create(NewRun {
                title: "second".into(),
                project: "/work".into(),
            })
            .unwrap();
        let updates = [(first.id, Stage::Build), (second.id, Stage::Check)];
        let threads: Vec<_> = updates
            .into_iter()
            .map(|(id, stage)| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .update(
                            &id,
                            UpdateRun {
                                stage,
                                status: RunStatus::Done,
                                activity: FieldUpdate::Keep,
                                current_ticket: FieldUpdate::Keep,
                                blocking_reason: FieldUpdate::Keep,
                                rework: FieldUpdate::Keep,
                            },
                        )
                        .unwrap()
                        .unwrap()
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let persisted = load_runs(&dir.0.path().join("runs.json")).unwrap();
        assert_eq!(persisted.len(), 2);
        assert!(persisted
            .iter()
            .all(|run| matches!(run.status, RunStatus::Done)));
        assert!(matches!(
            persisted
                .iter()
                .find(|run| run.title == "first")
                .unwrap()
                .stage,
            Stage::Build
        ));
        assert!(matches!(
            persisted
                .iter()
                .find(|run| run.title == "second")
                .unwrap()
                .stage,
            Stage::Check
        ));
    }

    #[test]
    fn cli_update_options_preserve_set_clear_and_reject_conflicts() {
        let update = parse_update("build", "running", &[]).unwrap();
        assert_eq!(
            serde_json::to_value(update).unwrap(),
            serde_json::json!({"stage":"build","status":"running"})
        );
        let update = parse_update(
            "build",
            "blocked",
            &[
                "--activity",
                "Implementing ticket",
                "--current-ticket",
                "https://example.com/42",
                "--blocking-reason",
                "Need input",
            ],
        )
        .unwrap();
        let value = serde_json::to_value(update).unwrap();
        assert_eq!(value["activity"], "Implementing ticket");
        assert_eq!(value["current_ticket"], "https://example.com/42");
        assert_eq!(value["blocking_reason"], "Need input");
        let update = parse_update(
            "finish",
            "done",
            &[
                "--clear-activity",
                "--clear-current-ticket",
                "--clear-blocking-reason",
            ],
        )
        .unwrap();
        let value = serde_json::to_value(update).unwrap();
        for field in ["activity", "current_ticket", "blocking_reason"] {
            assert_eq!(value.get(field), Some(&serde_json::Value::Null));
        }
        for options in [
            vec!["--activity"],
            vec!["--activity", "--clear-activity"],
            vec!["--clear-activity", "--activity", "x"],
            vec!["--clear-current-ticket", "--clear-current-ticket"],
            vec!["--unknown"],
            vec!["--current-ticket", "file:///tmp/ticket"],
            vec!["--blocking-reason", ""],
        ] {
            assert!(
                parse_update("build", "running", &options).is_err(),
                "options: {options:?}"
            );
        }
        assert!(parse_update("unknown", "running", &[]).is_err());
        assert!(parse_update("build", "unknown", &[]).is_err());
    }

    #[test]
    fn cli_rework_option_sets_clears_and_rejects_invalid_counts() {
        let update = parse_update("build", "running", &["--rework", "2"]).unwrap();
        assert_eq!(serde_json::to_value(update).unwrap()["rework"], 2);
        let update = parse_update("check", "running", &["--clear-rework"]).unwrap();
        assert_eq!(
            serde_json::to_value(update).unwrap().get("rework"),
            Some(&serde_json::Value::Null)
        );
        for options in [
            vec!["--rework"],
            vec!["--rework", "0"],
            vec!["--rework", "-1"],
            vec!["--rework", "two"],
            vec!["--rework", "1.5"],
            vec!["--rework", "100"],
            vec!["--rework", "1", "--clear-rework"],
            vec!["--rework", "1", "--rework", "2"],
        ] {
            assert!(
                parse_update("build", "running", &options).is_err(),
                "options: {options:?}"
            );
        }
        let range = format!("1 to {MAX_REWORK}");
        for count in ["0", "100", "two"] {
            let Err(error) = parse_update("build", "running", &["--rework", count]) else {
                panic!("--rework {count} should be rejected");
            };
            assert!(error.to_string().contains(&range), "{error}");
        }
        assert!(UPDATE_HELP.contains(&format!("--rework <1-{MAX_REWORK}>")));
    }

    #[test]
    fn failed_save_rolls_back_metadata_and_timestamp() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = store
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
            })
            .unwrap();
        let original = fs::read(&path).unwrap();
        fs::create_dir(path.with_extension("json.tmp")).unwrap();
        let update = parse_update(
            "build",
            "blocked",
            &["--activity", "Working", "--blocking-reason", "Need input"],
        )
        .unwrap();
        assert!(store.update(&run.id, update).is_err());
        assert_eq!(
            serde_json::to_vec(&store.list().unwrap()).unwrap(),
            original
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
