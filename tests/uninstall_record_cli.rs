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
