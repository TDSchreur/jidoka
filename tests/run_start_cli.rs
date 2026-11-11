//! `jidoka run start` reads the model-routing policy before it contacts the dashboard, so a mistake in
//! the file stops the start, and so does a policy for several agents without `--agent`. These tests only
//! check starts that fail before the dashboard is contacted, so they never reach one.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

mod common;

use std::fs;

#[test]
fn a_mistake_in_the_policy_stops_the_start_and_names_the_file() {
    let config = tempfile::tempdir().unwrap();
    let folder = config.path().join("jidoka");
    fs::create_dir(&folder).unwrap();
    let policy = folder.join("model-routing.json");
    fs::write(
        &policy,
        r#"{"agents":{"claude":{"stages":{"check":{"model":"son net"}}}}}"#,
    )
    .unwrap();
    let output = common::jidoka_in(config.path())
        .args(["run", "start", "never started"])
        .env("XDG_CONFIG_HOME", config.path())
        .env("APPDATA", config.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains(&policy.display().to_string()), "{error}");
    assert!(error.contains("Model"), "{error}");
}

/// `jidoka run start` with a temporary configuration folder holding this policy.
fn start(
    policy: &str,
    args: &[&str],
) -> (std::process::Output, std::path::PathBuf, tempfile::TempDir) {
    let config = tempfile::tempdir().unwrap();
    let folder = config.path().join("jidoka");
    fs::create_dir(&folder).unwrap();
    let path = folder.join("model-routing.json");
    fs::write(&path, policy).unwrap();
    let output = common::jidoka_in(config.path())
        .args(["run", "start", "never started"])
        .args(args)
        .env("XDG_CONFIG_HOME", config.path())
        .env("APPDATA", config.path())
        .output()
        .unwrap();
    (output, path, config)
}

const BOTH: &str = r#"{"agents":{
    "claude":{"stages":{"check":{"model":"opus"}}},
    "codex":{"stages":{"check":{"model":"gpt-6-astra"}}}}}"#;

#[test]
fn a_policy_for_several_agents_needs_the_agent_option_and_says_how_to_give_it() {
    let (output, path, _config) = start(BOTH, &[]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "claude, codex",
        "--agent claude",
        "--agent codex",
        "model-routing.json",
    ] {
        assert!(error.contains(expected), "{error}");
    }
    // The file is only read, never rewritten.
    assert_eq!(fs::read_to_string(path).unwrap(), BOTH);
}
