//! The startup check hints on stderr when Setup is missing and adopts existing installs.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]
use std::{
    path::Path,
    process::{Command, Output},
};

fn jidoka(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jidoka"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("APPDATA", home.join("config"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("COPILOT_HOME")
        .output()
        .unwrap()
}

#[test]
fn a_command_without_setup_hints_on_stderr_and_keeps_stdout() {
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(home.path(), &["run", "inspect", "x"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches("jidoka install").count(), 1, "{stderr}");
    assert!(output.stdout.is_empty());
}

#[test]
fn exempt_commands_and_adopted_installs_stay_quiet() {
    let home = tempfile::tempdir().unwrap();
    let version = jidoka(home.path(), &["--version"]);
    assert!(!String::from_utf8_lossy(&version.stderr).contains("jidoka install"));
    assert!(
        jidoka(home.path(), &["install", "--agent", "claude"])
            .status
            .success()
    );
    let output = jidoka(home.path(), &["run", "inspect", "x"]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("not set up"));
}
