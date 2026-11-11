//! `jidoka doctor` as a user runs it: exit status, JSON and argument errors.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

mod common;

use std::process::Output;

fn jidoka(args: &[&str], dir: &std::path::Path, home: &std::path::Path) -> Output {
    common::jidoka_in(home)
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("PATH", "")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_CODE_SUBAGENT_MODEL_FORCE")
        .env_remove("CLAUDE_CODE_SUBAGENT_MODEL")
        .env_remove("CLAUDE_CODE_EFFORT_LEVEL")
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("APPDATA", home.join(".config"))
        .output()
        .unwrap()
}

#[test]
fn doctor_fails_with_a_json_report_when_nothing_is_set_up() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(&["doctor", "--json"], dir.path(), home.path());
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["agents"].as_array().unwrap().len(), 3);
}

#[test]
fn doctor_with_an_agent_reports_only_that_agent() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(
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
    let output = jidoka(&["doctor", "--agent", "gemini"], dir.path(), home.path());
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown agent"));
}

#[test]
fn doctor_creates_the_model_routing_policy_once_and_keeps_json_output_valid() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let policy = home.path().join(".config/jidoka/model-routing.json");
    let run = || {
        jidoka(
            &[
                "doctor",
                "--agent",
                "claude",
                "--create-model-routing",
                "--json",
            ],
            dir.path(),
            home.path(),
        )
    };

    let first = run();
    // Stdout is still one JSON report, and the note about the file is on stderr.
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["model_routing"][0]["id"], "policy");
    assert_eq!(report["model_routing"][0]["status"], "pass");
    let note = String::from_utf8_lossy(&first.stderr);
    assert!(
        note.contains("Created") && note.contains("model-routing.json"),
        "{note}"
    );
    let text = std::fs::read_to_string(&policy).unwrap();
    assert!(
        text.contains("\"claude\"") && !text.contains("\"codex\""),
        "{text}"
    );

    // A second run, even after an edit, leaves the file as it is.
    std::fs::write(&policy, "{\"agents\":{\"codex\":{\"stages\":{}}}}").unwrap();
    let second = run();
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
    assert!(std::fs::read_to_string(&policy).unwrap().contains("codex"));
}

#[test]
fn doctor_creates_one_file_for_every_agent_and_checks_each_of_them() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(
        &["doctor", "--create-model-routing", "--json"],
        dir.path(),
        home.path(),
    );
    let note = String::from_utf8_lossy(&output.stderr);
    assert!(note.contains("claude, codex and copilot"), "{note}");
    let written =
        std::fs::read_to_string(home.path().join(".config/jidoka/model-routing.json")).unwrap();
    assert!(
        ["claude", "codex", "copilot"]
            .iter()
            .all(|agent| written.contains(&format!("\"{agent}\""))),
        "{written}"
    );
    // The next report checks both entries, each tagged with its agent.
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let agents: std::collections::BTreeSet<&str> = report["model_routing"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|check| check["agent"].as_str())
        .collect();
    assert_eq!(agents, ["claude", "codex", "copilot"].into());
    // Asking for one agent checks only its entry.
    let output = jidoka(
        &["doctor", "--agent", "codex", "--json"],
        dir.path(),
        home.path(),
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["model_routing"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| check["agent"].is_null() || check["agent"] == "codex")
    );
}
