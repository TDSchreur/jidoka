//! Which repository a folder belongs to, the same for the main checkout and every linked worktree.

use crate::doctor::git_output;
use std::path::Path;

/// The canonical repository path of `dir`: the main checkout, found through git's common directory.
/// `None` when `dir` is not in a repository or git cannot be run, so the caller records nothing.
pub(crate) fn canonical_repository(dir: &Path) -> Option<String> {
    let common = git_output(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = Path::new(&common);
    // A normal repository keeps its data in `<checkout>/.git`; a bare one is the directory itself.
    let root = if common.file_name().is_some_and(|name| name == ".git") {
        common.parent()?
    } else {
        common
    };
    // Git always prints `/`; the native spelling matches the folder a run reports on Windows.
    Some(
        root.components()
            .collect::<std::path::PathBuf>()
            .display()
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn a_linked_worktree_has_the_identity_of_its_main_checkout() {
        let base = tempfile::tempdir().unwrap();
        let main = base.path().join("repo");
        std::fs::create_dir(&main).unwrap();
        git(&main, &["init", "-q"]);
        git(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let linked = base.path().join("repo.worktrees").join("feature");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                linked.to_str().unwrap(),
            ],
        );
        let sub = linked.join("nested");
        std::fs::create_dir(&sub).unwrap();

        let from_main = canonical_repository(&main).expect("main checkout");
        assert_eq!(canonical_repository(&linked), Some(from_main.clone()));
        assert_eq!(canonical_repository(&sub), Some(from_main.clone()));
        let expected = main.canonicalize().unwrap();
        assert_eq!(Path::new(&from_main).canonicalize().unwrap(), expected);
        assert!(!from_main.ends_with(".git"));
    }

    #[test]
    fn a_folder_outside_any_repository_has_no_identity() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(canonical_repository(dir.path()), None);
    }
}
