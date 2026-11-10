//! `factory run start` reads the model-routing policy before it contacts the dashboard, so a mistake in
//! the file stops the start, and so does a policy for several agents without `--agent`. These tests only
//! check starts that fail before the dashboard is contacted, so they never reach one.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

use std::{fs, process::Command};

#[test]
fn a_mistake_in_the_policy_stops_the_start_and_names_the_file() {
    let config = tempfile::tempdir().unwrap();
    let folder = config.path().join("tds-factory");
    fs::create_dir(&folder).unwrap();
    let policy = folder.join("model-routing.json");
    fs::write(
        &policy,
        r#"{"agents":{"claude":{"stages":{"check":{"model":"son net"}}}}}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_factory"))
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

/// `factory run start` with a temporary configuration folder holding this policy.
fn start(
    policy: &str,
    args: &[&str],
) -> (std::process::Output, std::path::PathBuf, tempfile::TempDir) {
    let config = tempfile::tempdir().unwrap();
    let folder = config.path().join("tds-factory");
    fs::create_dir(&folder).unwrap();
    let path = folder.join("model-routing.json");
    fs::write(&path, policy).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_factory"))
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

#[test]
fn a_mistake_in_one_agents_entry_stops_a_start_for_either_agent() {
    let broken = BOTH.replace("gpt-6-astra", "gpt 6");
    for agent in ["claude", "codex"] {
        let (output, _path, _config) = start(&broken, &["--agent", agent]);
        assert!(!output.status.success(), "{agent}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("agents.codex") && error.contains("Model"),
            "{agent}: {error}"
        );
    }
}

#[test]
fn an_old_format_policy_stops_the_start_and_says_how_to_replace_it() {
    let old = [
        r#"{"agents":{"claude":{"roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}}}}"#,
        r#"{"agent":"claude","roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}}"#,
    ];
    for policy in old {
        let (output, path, _config) = start(policy, &["--agent", "claude"]);
        assert!(!output.status.success(), "{policy}");
        let error = String::from_utf8_lossy(&output.stderr);
        for expected in [
            "model-routing.json",
            "old format",
            "factory install",
            "factory doctor --create-model-routing",
        ] {
            assert!(error.contains(expected), "{policy}: {error}");
        }
        // The file is only read, never rewritten.
        assert_eq!(fs::read_to_string(path).unwrap(), policy);
    }
}

#[test]
fn a_bad_agent_option_is_refused_before_anything_is_read_or_started() {
    for (args, expected) in [
        (&["--agent"][..], "needs"),
        (&["--agent", "gemini"], "Unknown agent"),
        (&["--agent", "claude", "--agent", "codex"], "Unexpected"),
        (&["--session", "x"], "Unexpected"),
    ] {
        let (output, _path, _config) = start("this is not even JSON", args);
        assert!(!output.status.success(), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "{args:?}: {error}");
        // The usage text shows the option.
        assert!(
            error.contains("factory run start <title> [--agent"),
            "{error}"
        );
        assert!(
            !error.contains("not valid JSON"),
            "the policy was read first: {error}"
        );
    }
}
