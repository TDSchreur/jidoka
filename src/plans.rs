//! Where a run's plan is saved: in the Jidoka data dir, per run, so it outlives the run's worktree.

use std::{
    io,
    path::{Path, PathBuf},
};

/// The folder holding the plans that belong to the runs file at `runs`: `plans` beside `runs.json`.
pub(crate) fn plans_dir(runs: &Path) -> PathBuf {
    crate::server::data_dir(runs).join("plans")
}

/// `<plans dir>/<run id>.md`; nothing is created.
pub(crate) fn plan_path(runs: &Path, id: &str) -> PathBuf {
    plans_dir(runs).join(format!("{id}.md"))
}

/// The plan path of a run, with its folder created (private, like the data dir) so the file can be written.
pub(crate) fn plan_path_ready(runs: &Path, id: &str) -> io::Result<PathBuf> {
    let dir = plans_dir(runs);
    create_private(&dir)?;
    Ok(plan_path(runs, id))
}

#[cfg(unix)]
fn create_private(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if std::fs::symlink_metadata(dir).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::other(format!(
            "the plans folder {} is a symlink; remove it so plans stay in the Jidoka data folder",
            dir.display()
        )));
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_runs_file_keeps_its_plans_in_a_plans_folder_beside_it() {
        let runs = Path::new("/data/jidoka/runs.json");
        assert_eq!(plan_path(runs, "7"), Path::new("/data/jidoka/plans/7.md"));
    }

    #[test]
    fn the_plans_folder_is_created_lazily() {
        let data = tempfile::tempdir().unwrap();
        let runs = data.path().join("runs.json");
        let path = plan_path(&runs, "3");
        assert!(!path.parent().unwrap().exists());
        let ready = plan_path_ready(&runs, "3").unwrap();
        assert_eq!(ready, path);
        assert!(ready.parent().unwrap().is_dir());
        assert!(!ready.exists());
    }
}
