//! Installation creates the default policy without replacing user settings.
mod common;

use std::fs;

#[test]
fn install_creates_the_two_agent_policy_and_preserves_existing_content() {
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
                ("define", "sonnet", "low"),
                ("build", "sonnet", "low"),
                ("check", "sonnet", "medium"),
                ("finish", "haiku", "low"),
            ],
        ),
        (
            "codex",
            [
                ("define", "gpt-6.1-terra", "low"),
                ("build", "gpt-6.1-terra", "low"),
                ("check", "gpt-6.1-terra", "medium"),
                ("finish", "gpt-6-luna", "low"),
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
    assert_eq!(fs::read_to_string(&path).unwrap(), "my existing policy");
}
