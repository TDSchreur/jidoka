//! Uninstall operates on selected installed skills and preserves unattended data.
use std::{fs, process::Command};

// On Windows the command removes LOCALAPPDATA\Programs\jidoka from the real user PATH. Tests point
// LOCALAPPDATA at a temporary directory that is never on PATH, so the registry is only read, never written.

#[test]
fn removes_selected_skills_and_preserves_other_files_and_data() {
    let home = tempfile::tempdir().unwrap();
    let codex = home.path().join("codex");
    let skills = codex.join("skills");
    let data = home.path().join("data/jidoka");
    fs::create_dir_all(skills.join("jidoka")).unwrap();
    fs::create_dir_all(skills.join("other")).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::write(skills.join("jidoka/SKILL.md"), "installed").unwrap();
    fs::write(skills.join("other/SKILL.md"), "mine").unwrap();
    fs::write(data.join("runs.json"), "runs").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jidoka"))
        .args(["uninstall", "--agent", "codex", "--keep-data"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("CODEX_HOME", &codex)
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("LOCALAPPDATA", home.path().join("data"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!skills.join("jidoka").exists());
    assert!(skills.join("other/SKILL.md").exists());
    assert!(data.join("runs.json").exists());
}

#[test]
fn custom_claude_selection_and_noninteractive_input_preserve_data() {
    let home = tempfile::tempdir().unwrap();
    let custom = home.path().join("custom");
    let default = home.path().join(".claude/skills/jidoka");
    let selected = custom.join("skills/jidoka");
    let data = home.path().join("data/jidoka/runs.json");
    fs::create_dir_all(&selected).unwrap();
    fs::create_dir_all(&default).unwrap();
    fs::create_dir_all(data.parent().unwrap()).unwrap();
    fs::write(&data, "history").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_jidoka"))
        .args(["uninstall", "--agent", "claude", "--config-dir"])
        .arg(&custom)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("LOCALAPPDATA", home.path().join("data"))
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    // Non-interactive input is never read, so the child may exit before the write.
    if let Err(error) = child.stdin.take().unwrap().write_all(b"yes\n") {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
    assert!(child.wait().unwrap().success());
    assert!(!selected.exists());
    assert!(default.exists());
    assert!(data.exists());
}

#[cfg(unix)]
#[test]
fn removes_only_the_installer_location_not_an_unrelated_path_command() {
    let home = tempfile::tempdir().unwrap();
    let installed = home.path().join(".local/bin/jidoka");
    let unrelated = home.path().join("other/jidoka");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
    fs::write(&installed, "installer copy").unwrap();
    fs::write(&unrelated, "other command").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jidoka"))
        .args(["uninstall", "--agent", "codex", "--keep-data"])
        .env("HOME", home.path())
        .env("CODEX_HOME", home.path().join("codex"))
        .env("PATH", unrelated.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!installed.exists());
    assert!(unrelated.exists());
}

#[cfg(windows)]
#[test]
fn removes_installer_copy_and_preserves_unrelated_command() {
    let home = tempfile::tempdir().unwrap();
    let installed = home.path().join("data/Programs/jidoka/jidoka.exe");
    let unrelated = home.path().join("other/jidoka.exe");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
    fs::write(&installed, "installer copy").unwrap();
    fs::write(&unrelated, "other command").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jidoka"))
        .args(["uninstall", "--agent", "codex", "--keep-data"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("CODEX_HOME", home.path().join("codex"))
        .env("LOCALAPPDATA", home.path().join("data"))
        .env("PATH", unrelated.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "uninstall failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !installed.exists(),
        "installer copy was not removed: {}",
        installed.display()
    );
    assert!(
        unrelated.exists(),
        "unrelated command was removed: {}",
        unrelated.display()
    );
}

#[cfg(unix)]
#[test]
fn reports_uninspectable_installer_location() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join(".local"), "not a directory").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jidoka"))
        .args(["uninstall", "--agent", "codex", "--keep-data"])
        .env("HOME", home.path())
        .env("CODEX_HOME", home.path().join("codex"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot inspect"));
}
