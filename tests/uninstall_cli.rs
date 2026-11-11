//! Uninstall operates on selected installed skills and preserves unattended data.
mod common;

use std::fs;

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
    let output = common::jidoka_in(home.path())
        .args(["uninstall", "--agent", "codex", "--keep-data"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("APPDATA", home.path().join("config"))
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
    let mut child = common::jidoka_in(home.path())
        .args(["uninstall", "--agent", "claude", "--config-dir"])
        .arg(&custom)
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("APPDATA", home.path().join("config"))
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
fn leaves_installer_copy_and_path_command_when_they_are_not_the_running_binary() {
    let home = tempfile::tempdir().unwrap();
    let installed = home.path().join(".local/bin/jidoka");
    let unrelated = home.path().join("other/jidoka");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
    fs::write(&installed, "installer copy").unwrap();
    fs::write(&unrelated, "other command").unwrap();
    let output = common::jidoka_in(home.path())
        .args(["uninstall", "--agent", "codex", "--keep-data"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("APPDATA", home.path().join("config"))
        .env("CODEX_HOME", home.path().join("codex"))
        .env("PATH", unrelated.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(installed.exists());
    assert!(unrelated.exists());
    assert!(String::from_utf8_lossy(&output.stdout).contains("package manager"));
}

#[cfg(windows)]
#[test]
fn leaves_installer_copy_and_unrelated_command_when_they_are_not_the_running_binary() {
    let home = tempfile::tempdir().unwrap();
    let installed = home.path().join("data/Programs/jidoka/jidoka.exe");
    let unrelated = home.path().join("other/jidoka.exe");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
    fs::write(&installed, "installer copy").unwrap();
    fs::write(&unrelated, "other command").unwrap();
    let output = common::jidoka_in(home.path())
        .args(["uninstall", "--agent", "codex", "--keep-data"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("APPDATA", home.path().join("config"))
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
        installed.exists(),
        "installer copy was removed: {}",
        installed.display()
    );
    assert!(
        unrelated.exists(),
        "unrelated command was removed: {}",
        unrelated.display()
    );
}
