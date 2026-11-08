//! Keeps entries in a worktree's `.gitignore`.

use std::{fs, io, path::Path};

/// The form of a `.gitignore` line that decides what it ignores, or `None` for a blank line, a comment or a negation.
fn ignored_name(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
        return None;
    }
    Some(line.trim_start_matches('/').trim_end_matches('/'))
}

/// Appends the `entries` that `<worktree>/.gitignore` does not list yet (`.factory`, `/.factory` and `.factory/`
/// count alike), creating the file when it is absent. Returns the entries it added; nothing is committed.
pub(crate) fn ensure_ignored(worktree: &Path, entries: &[&str]) -> io::Result<Vec<String>> {
    let path = worktree.join(".gitignore");
    let mut content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let missing: Vec<String> = entries
        .iter()
        .filter(|entry| {
            let wanted = entry.trim_matches('/');
            !content
                .lines()
                .any(|line| ignored_name(line) == Some(wanted))
        })
        .map(|entry| entry.to_string())
        .collect();
    if missing.is_empty() {
        return Ok(missing);
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    for entry in &missing {
        content.push_str(entry);
        content.push('\n');
    }
    fs::write(&path, content)?;
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRIES: &[&str] = &[".factory/", ".scratch/"];

    fn read(dir: &Path) -> String {
        fs::read_to_string(dir.join(".gitignore")).unwrap()
    }

    #[test]
    fn only_the_missing_entry_is_added() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "a\n.factory/\n").unwrap();
        assert_eq!(ensure_ignored(dir.path(), ENTRIES).unwrap(), [".scratch/"]);
        assert_eq!(read(dir.path()), "a\n.factory/\n.scratch/\n");
    }

    #[test]
    fn spelling_variants_count_as_present_and_leave_the_file_alone() {
        for text in [
            ".factory\n.scratch\n",
            "/.factory\n/.scratch/\n",
            "/.factory/\n  .scratch/  \n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join(".gitignore"), text).unwrap();
            assert!(ensure_ignored(dir.path(), ENTRIES).unwrap().is_empty());
            assert_eq!(read(dir.path()), text);
        }
    }

    #[test]
    fn a_comment_or_negation_does_not_count_and_a_missing_newline_is_added() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "# .factory/\n!.scratch/\nx").unwrap();
        ensure_ignored(dir.path(), ENTRIES).unwrap();
        assert_eq!(
            read(dir.path()),
            "# .factory/\n!.scratch/\nx\n.factory/\n.scratch/\n"
        );
    }

    #[test]
    fn calling_it_twice_adds_nothing_the_second_time() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ensure_ignored(dir.path(), ENTRIES).unwrap().len(), 2);
        assert!(ensure_ignored(dir.path(), ENTRIES).unwrap().is_empty());
        assert_eq!(read(dir.path()), ".factory/\n.scratch/\n");
    }
}
