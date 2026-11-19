//! Install and doctor resolve one agent home through its native override.
#![allow(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]
mod common;

#[test]
fn install_and_doctor_use_each_agents_native_home() {
    let home = tempfile::tempdir().unwrap();
    let overrides = [
        ("codex", "CODEX_HOME", home.path().join("codex-work")),
        (
            "claude",
            "CLAUDE_CONFIG_DIR",
            home.path().join("claude-work"),
        ),
        ("copilot", "COPILOT_HOME", home.path().join("copilot-work")),
    ];
    let mut install = common::jidoka_in(home.path());
    let output = install
        .envs(overrides.iter().map(|(_, name, path)| (name, path)))
        .args(["install", "--agent", "all"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let mut doctor = common::jidoka_in(home.path());
    doctor.env("PATH", home.path());
    let output = doctor
        .envs(overrides.iter().map(|(_, name, path)| (name, path)))
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let agents = report["agents"].as_array().unwrap();
    assert_eq!(agents.len(), overrides.len(), "{report}");
    for (name, _, path) in overrides {
        assert!(path.join("skills/jidoka/SKILL.md").is_file(), "{path:?}");
        let agent = agents.iter().find(|agent| agent["id"] == name).unwrap();
        let skills: Vec<_> = agent["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|check| check["id"] == "skills")
            .collect();
        assert_eq!(skills.len(), 1, "{agent}");
        assert_eq!(skills[0]["status"], "pass");
        assert!(
            skills[0]["message"]
                .as_str()
                .unwrap()
                .contains(path.to_str().unwrap())
        );
    }
}

#[test]
fn install_and_doctor_reject_the_removed_config_dir_flag() {
    let home = tempfile::tempdir().unwrap();
    let extra = home.path().join("extra");
    for command in ["install", "doctor"] {
        let output = common::jidoka_in(home.path())
            .args([command, "--agent", "claude", "--config-dir"])
            .arg(&extra)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{command}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--config-dir"),
            "{command}: {output:?}"
        );
    }
    assert!(!extra.exists());
    assert!(!home.path().join(".claude/skills").exists());
}
