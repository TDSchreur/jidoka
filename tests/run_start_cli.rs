//! `factory run start` reads the model-routing policy before it contacts the dashboard, so a mistake in
//! the file stops the start. These tests never reach a dashboard.

use std::{fs, process::Command};

#[test]
fn a_mistake_in_the_policy_stops_the_start_and_names_the_file() {
    let config = tempfile::tempdir().unwrap();
    let folder = config.path().join("tds-factory");
    fs::create_dir(&folder).unwrap();
    let policy = folder.join("model-routing.json");
    fs::write(
        &policy,
        r#"{"agent":"claude","roles":{"coding":{"model":"sonnet"}},"tasks":{"check":"strong"}}"#,
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
    assert!(error.contains("strong"), "{error}");
}
