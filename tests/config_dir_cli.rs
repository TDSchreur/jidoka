//! `--config-dir` installs the Claude skills into each named folder and leaves `~/.claude` alone.
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
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("COPILOT_HOME")
        .env_remove("JIDOKA_CLAUDE_CONFIG_DIRS")
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn install_writes_every_config_dir_once_and_not_the_default() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (home.path().join("a"), home.path().join("b"));
    let (a_arg, b_arg) = (a.to_str().unwrap(), b.to_str().unwrap());
    let output = jidoka(
        home.path(),
        &[
            "install",
            "--agent",
            "claude",
            "--config-dir",
            a_arg,
            "--config-dir",
            b_arg,
            "--config-dir",
            a_arg,
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    for dir in [&a, &b] {
        assert!(dir.join("skills/jidoka/SKILL.md").is_file(), "{dir:?}");
    }
    assert!(!home.path().join(".claude").exists());
}

#[test]
fn a_relative_config_dir_fails_before_anything_is_written() {
    let home = tempfile::tempdir().unwrap();
    let good = home.path().join("good");
    let output = jidoka(
        home.path(),
        &[
            "install",
            "--agent",
            "claude",
            "--config-dir",
            good.to_str().unwrap(),
            "--config-dir",
            "relative/dir",
        ],
    );
    assert!(!output.status.success());
    assert!(stderr(&output).contains("absolute"), "{}", stderr(&output));
    assert!(!good.exists());
    assert!(!home.path().join("relative").exists());
    assert!(!home.path().join(".claude").exists());
}

#[test]
fn doctor_reports_each_config_dir_by_path() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (home.path().join("a"), home.path().join("b"));
    let install = jidoka(
        home.path(),
        &[
            "install",
            "--agent",
            "claude",
            "--config-dir",
            a.to_str().unwrap(),
        ],
    );
    assert!(install.status.success(), "{}", stderr(&install));
    let output = jidoka(
        home.path(),
        &[
            "doctor",
            "--agent",
            "claude",
            "--json",
            "--config-dir",
            a.to_str().unwrap(),
            "--config-dir",
            b.to_str().unwrap(),
        ],
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let checks = report["agents"][0]["checks"].as_array().unwrap();
    let skills: Vec<_> = checks.iter().filter(|c| c["id"] == "skills").collect();
    assert_eq!(skills.len(), 2, "{report}");
    assert_eq!(skills[0]["status"], "pass");
    assert!(
        skills[0]["message"]
            .as_str()
            .unwrap()
            .contains(a.to_str().unwrap())
    );
    assert_eq!(skills[1]["status"], "missing");
    assert!(
        skills[1]["message"]
            .as_str()
            .unwrap()
            .contains(b.to_str().unwrap())
    );
}

fn record(home: &Path) -> serde_json::Value {
    let text = std::fs::read(home.join("config/jidoka/install.json")).unwrap();
    serde_json::from_slice(&text).unwrap()
}

#[test]
fn install_with_an_agent_writes_the_skills_and_the_record() {
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(home.path(), &["install", "--agent", "claude"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(home.path().join(".claude/skills/jidoka/SKILL.md").is_file());
    let record = record(home.path());
    assert_eq!(record["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(record["agents"], serde_json::json!(["claude"]));
    assert_eq!(record["claude_config_dirs"], serde_json::json!([]));
}
