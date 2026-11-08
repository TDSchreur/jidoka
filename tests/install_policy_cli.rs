//! Installation creates the default policy without replacing user settings.
use std::{fs, process::Command};

#[test]
fn install_creates_the_two_agent_policy_and_preserves_existing_content() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_factory"))
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
    let path = config.join("tds-factory/model-routing.json");
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

const OLD_MULTI: &str =
    r#"{"agents":{"claude":{"roles":{"fast":{"model":"haiku"}},"tasks":{"build":"fast"}}}}"#;
const OLD_SINGLE: &str =
    r#"{"agent":"claude","roles":{"fast":{"model":"haiku"}},"tasks":{"build":"fast"}}"#;
const CURRENT: &str = r#"{"agents":{"codex":{"stages":{"build":{"model":"gpt-6.1-terra"}}}}}"#;

/// Runs `factory install` with `existing` already at the policy path; returns the file and stdout.
fn install_over(existing: &str) -> (String, String, std::path::PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let path = config.join("tds-factory/model-routing.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, existing).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(["install", "--agent", "codex"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("APPDATA", &config)
        .env("CODEX_HOME", home.path().join("codex"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&path).unwrap();
    (
        text,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        path,
    )
}

#[test]
fn install_replaces_an_old_format_policy_in_either_form_and_says_so() {
    for old in [OLD_MULTI, OLD_SINGLE] {
        let (text, stdout, path) = install_over(old);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["agents"]["claude"]["stages"]["build"]["model"],
            "sonnet"
        );
        assert!(value["agents"]["codex"]["stages"]["define"].is_object());
        assert!(
            stdout.contains("Replaced old-format model routing policy")
                && stdout.contains(&path.display().to_string()),
            "{stdout}"
        );
    }
}

#[test]
fn install_replaces_a_switched_off_old_policy_and_keeps_the_switches_off() {
    let (text, stdout, _) = install_over(
        r#"{"enabled":false,"agents":{"codex":{"enabled":false,"roles":{}},"claude":{"roles":{}}}}"#,
    );
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["enabled"], false);
    assert_eq!(value["agents"]["codex"]["enabled"], false);
    assert!(value["agents"]["claude"].get("enabled").is_none());
    assert!(value["agents"]["claude"]["stages"]["build"].is_object());
    assert!(stdout.contains("Replaced old-format"), "{stdout}");
}

#[test]
fn install_leaves_a_current_format_or_unrecognised_policy_alone() {
    for existing in [CURRENT, "not json at all", r#"{"something":"else"}"#, ""] {
        let (text, stdout, _) = install_over(existing);
        assert_eq!(text, existing);
        assert!(!stdout.contains("Replaced"), "{stdout}");
    }
}
