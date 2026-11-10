//! `jidoka uninstall` follows the install record and leaves a binary it did not install.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]
mod common;

use std::{path::Path, process::Output};

fn jidoka(home: &Path, args: &[&str]) -> Output {
    common::jidoka_in(home)
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("APPDATA", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("LOCALAPPDATA", home.join("data"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("COPILOT_HOME")
        .env_remove("JIDOKA_AGENT")
        .env_remove("JIDOKA_CLAUDE_CONFIG_DIRS")
        .output()
        .unwrap()
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
    assert!(stdout.contains("winget uninstall"), "{stdout}");
}

#[test]
fn uninstall_without_record_or_agent_names_the_flag() {
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(home.path(), &["uninstall"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--agent"));
}
