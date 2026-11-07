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
    assert_eq!(
        value["agents"]["claude"]["roles"]["strong"]["model"],
        "sonnet"
    );
    assert_eq!(
        value["agents"]["codex"]["roles"]["strong"]["model"],
        "gpt-6.1-terra"
    );
    assert_eq!(value["agents"]["codex"]["tasks"]["check"], "strong");
    fs::write(&path, "my existing policy").unwrap();
    assert!(run().status.success());
    assert_eq!(fs::read_to_string(&path).unwrap(), "my existing policy");
}
