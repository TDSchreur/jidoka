//! `jidoka uninstall` follows the install record and leaves a binary it did not install.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]
mod common;

use std::{path::Path, process::Output};

fn jidoka(home: &Path, args: &[&str]) -> Output {
    common::jidoka_in(home).args(args).output().unwrap()
}

#[test]
fn uninstall_without_flags_uses_the_record_and_leaves_a_foreign_binary() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("cc");
    let dir_arg = dir.to_str().unwrap();
    assert!(
        jidoka(
            home.path(),
            &["install", "--agent", "claude", "--config-dir", dir_arg]
        )
        .status
        .success()
    );
    let record = home.path().join("config/jidoka/install.json");
    assert!(record.exists());
    let output = jidoka(home.path(), &["uninstall", "--keep-data"]);
    assert!(output.status.success(), "{output:?}");
    assert!(!record.exists());
    assert!(!dir.join("skills/jidoka").exists());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("package manager"), "{stdout}");
    assert_eq!(stdout.contains("winget"), cfg!(windows), "{stdout}");
}

#[test]
fn uninstall_without_record_or_agent_names_the_flag() {
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(home.path(), &["uninstall"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--agent"));
}

#[test]
fn a_corrupt_record_does_not_block_uninstall_with_an_agent() {
    let home = tempfile::tempdir().unwrap();
    let record = home.path().join("config/jidoka/install.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(&record, "{").unwrap();
    let blocked = jidoka(home.path(), &["uninstall", "--keep-data"]);
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(!blocked.status.success());
    assert!(
        stderr.contains("install.json") && stderr.contains("--agent"),
        "{stderr}"
    );
    let output = jidoka(
        home.path(),
        &["uninstall", "--agent", "claude", "--keep-data"],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(!record.exists());
}

fn install(home: &Path, agents: &str, dir: &Path) {
    let output = jidoka(
        home,
        &[
            "install",
            "--agent",
            agents,
            "--config-dir",
            dir.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn a_partial_uninstall_keeps_the_record_minus_the_agent() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("cc");
    install(home.path(), "claude,codex", &dir);
    let record = home.path().join("config/jidoka/install.json");
    let output = jidoka(home.path(), &["uninstall", "--agent", "codex"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("claude") && stdout.contains("remain"),
        "{stdout}"
    );
    let text = std::fs::read_to_string(&record).unwrap();
    assert!(text.contains("claude") && !text.contains("codex"), "{text}");
    assert!(dir.join("skills/jidoka").exists());
}

#[test]
fn a_partial_uninstall_of_one_config_dir_keeps_the_other() {
    let home = tempfile::tempdir().unwrap();
    let (one, two) = (home.path().join("one"), home.path().join("two"));
    let output = jidoka(
        home.path(),
        &[
            "install",
            "--agent",
            "claude",
            "--config-dir",
            one.to_str().unwrap(),
            "--config-dir",
            two.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let record = home.path().join("config/jidoka/install.json");
    let output = jidoka(
        home.path(),
        &[
            "uninstall",
            "--agent",
            "claude",
            "--config-dir",
            one.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let text = std::fs::read_to_string(&record).unwrap();
    assert!(text.contains("two") && !text.contains("one"), "{text}");
    assert!(two.join("skills/jidoka").exists());
}

#[test]
fn removing_the_last_agent_removes_the_record() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("cc");
    install(home.path(), "claude,codex", &dir);
    let record = home.path().join("config/jidoka/install.json");
    let first = jidoka(
        home.path(),
        &["uninstall", "--agent", "codex", "--keep-data"],
    );
    assert!(first.status.success(), "{first:?}");
    assert!(record.exists());
    let last = jidoka(
        home.path(),
        &["uninstall", "--agent", "claude", "--keep-data"],
    );
    assert!(last.status.success(), "{last:?}");
    assert!(!record.exists());
    let note = if cfg!(windows) {
        "winget uninstall"
    } else {
        "package manager"
    };
    assert!(String::from_utf8_lossy(&last.stdout).contains(note));
}

#[test]
fn an_agent_that_is_not_recorded_leaves_the_record_and_says_so() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("cc");
    install(home.path(), "claude", &dir);
    let record = home.path().join("config/jidoka/install.json");
    let before = std::fs::read_to_string(&record).unwrap();
    let output = jidoka(home.path(), &["uninstall", "--agent", "codex"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Not recorded") && stdout.contains("codex"),
        "{stdout}"
    );
    assert_eq!(std::fs::read_to_string(&record).unwrap(), before);
    assert!(dir.join("skills/jidoka").exists());
}
