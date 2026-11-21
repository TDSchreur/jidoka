//! Installation replaces the policy with defaults for all agents.
mod common;

use std::fs;

#[test]
fn install_replaces_the_policy_with_defaults_for_all_agents() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let run = || {
        common::jidoka_in(home.path())
            .args(["install", "--agent", "codex"])
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("XDG_CONFIG_HOME", &config)
            .env("APPDATA", &config)
            .env("CODEX_HOME", home.path().join("codex"))
            .output()
            .unwrap()
    };
    let output = run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = config.join("jidoka").join("model-routing.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["enabled"], true);
    for (agent, expected) in [
        (
            "claude",
            [
                ("define", "sonnet", "medium"),
                ("build", "sonnet", "medium"),
                ("check", "sonnet", "medium"),
                ("finish", "haiku", "low"),
            ],
        ),
        (
            "codex",
            [
                ("define", "gpt-6.2-sol", "medium"),
                ("build", "gpt-6.2-sol", "medium"),
                ("check", "gpt-6.2-astra", "medium"),
                ("finish", "gpt-6-luna", "low"),
            ],
        ),
        (
            "copilot",
            [
                ("define", "claude-sonnet-5.5", "medium"),
                ("build", "claude-sonnet-5.5", "medium"),
                ("check", "claude-sonnet-5.5", "medium"),
                ("finish", "claude-haiku-4.5", "low"),
            ],
        ),
    ] {
        for (stage, model, effort) in expected {
            let choice = &value["agents"][agent]["stages"][stage];
            assert_eq!(choice["model"], model, "{agent} {stage}");
            assert_eq!(choice["effort"], effort, "{agent} {stage}");
        }
    }
    fs::write(&path, "my existing policy").unwrap();
    assert!(run().status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
        value
    );
}
