//! Where a run's plan is saved: in the Jidoka data dir, per run, so it outlives the run's worktree.

use crate::server::DEFAULT_PORT;
use std::{
    io,
    path::{Path, PathBuf},
};

/// The folder holding the plans that belong to the runs file at `runs`: `plans` beside `runs.json`, or
/// `plans-<port>` beside `runs-<port>.json`, so a dashboard on another port shares no plans.
/// The default port has no suffix.
pub(crate) fn plans_dir(runs: &Path, port: u16) -> PathBuf {
    let name = if port == DEFAULT_PORT {
        "plans".to_string()
    } else {
        format!("plans-{port}")
    };
    runs.parent().unwrap_or_else(|| Path::new("")).join(name)
}

/// `<plans dir>/<run id>.md`; nothing is created.
pub(crate) fn plan_path(runs: &Path, port: u16, id: &str) -> PathBuf {
    plans_dir(runs, port).join(format!("{id}.md"))
}

/// The plan path of a run, with its folder created (private, like the data dir) so the file can be written.
pub(crate) fn plan_path_ready(runs: &Path, port: u16, id: &str) -> io::Result<PathBuf> {
    let dir = plans_dir(runs, port);
    create_private(&dir)?;
    Ok(plan_path(runs, port, id))
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
        assert_eq!(
            plan_path(runs, DEFAULT_PORT, "7"),
            Path::new("/data/jidoka/plans/7.md")
        );
    }

    #[test]
    fn another_port_keeps_its_plans_in_its_own_folder() {
        let runs = Path::new("/data/jidoka/runs-4400.json");
        assert_eq!(
            plan_path(runs, 4400, "7"),
            Path::new("/data/jidoka/plans-4400/7.md")
        );
    }

    #[test]
    fn the_plans_folder_is_created_lazily() {
        let data = tempfile::tempdir().unwrap();
        let runs = data.path().join("runs.json");
        let path = plan_path(&runs, DEFAULT_PORT, "3");
        assert!(!path.parent().unwrap().exists());
        let ready = plan_path_ready(&runs, DEFAULT_PORT, "3").unwrap();
        assert_eq!(ready, path);
        assert!(ready.parent().unwrap().is_dir());
        assert!(!ready.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_plans_folder_is_refused_and_its_target_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let data = tempfile::tempdir().unwrap();
        let target = data.path().join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runs = data.path().join("runs.json");
        std::os::unix::fs::symlink(&target, plans_dir(&runs, DEFAULT_PORT)).unwrap();
        let error = plan_path_ready(&runs, DEFAULT_PORT, "3").unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn the_plans_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let data = tempfile::tempdir().unwrap();
        let runs = data.path().join("jidoka/runs.json");
        let ready = plan_path_ready(&runs, DEFAULT_PORT, "3").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(ready.parent().unwrap()), 0o700);
    }
}
