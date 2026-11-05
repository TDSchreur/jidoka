//! `factory doctor` as a user runs it: exit status, JSON and argument errors.

use std::process::{Command, Output};

fn factory(args: &[&str], dir: &std::path::Path, home: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("PATH", "")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .unwrap()
}

#[test]
fn doctor_fails_with_a_json_report_when_nothing_is_set_up() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(&["doctor", "--json"], dir.path(), home.path());
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["agents"].as_array().unwrap().len(), 3);
}

#[test]
fn doctor_with_an_agent_reports_only_that_agent() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(
        &["doctor", "--agent", "codex", "--json"],
        dir.path(),
        home.path(),
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["agents"].as_array().unwrap().len(), 1);
    assert_eq!(report["agents"][0]["id"], "codex");
}

#[test]
fn doctor_rejects_unknown_arguments_without_a_report() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(&["doctor", "--agent", "gemini"], dir.path(), home.path());
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown agent"));
}

#[test]
fn help_lists_doctor() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(&["--help"], dir.path(), home.path());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("factory doctor [--agent <codex|claude|copilot>] [--json]"));
}
