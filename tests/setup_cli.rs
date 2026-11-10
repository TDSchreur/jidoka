//! The startup check hints on stderr when Setup is missing and adopts existing installs.
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
        .output()
        .unwrap()
}

#[test]
fn a_command_without_setup_hints_on_stderr_and_keeps_stdout() {
    let home = tempfile::tempdir().unwrap();
    let output = jidoka(home.path(), &["run", "inspect", "x"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches("jidoka install").count(), 1, "{stderr}");
    assert!(output.stdout.is_empty());
}

#[test]
fn exempt_commands_and_adopted_installs_stay_quiet() {
    let home = tempfile::tempdir().unwrap();
    let version = jidoka(home.path(), &["--version"]);
    assert!(!String::from_utf8_lossy(&version.stderr).contains("jidoka install"));
    assert!(
        jidoka(home.path(), &["install", "--agent", "claude"])
            .status
            .success()
    );
    let output = jidoka(home.path(), &["run", "inspect", "x"]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("not set up"));
}

#[test]
fn an_older_record_refreshes_the_skills_with_one_stderr_line_and_no_stdout() {
    let home = tempfile::tempdir().unwrap();
    let record = home.path().join("config/jidoka/install.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        r#"{"version":"0.0.1","agents":["claude"],"claude_config_dirs":[]}"#,
    )
    .unwrap();
    let before = jidoka(home.path(), &["run", "inspect", "x"]);
    let stderr = String::from_utf8_lossy(&before.stderr);
    let expected = format!(
        "jidoka: updated skills to {} for claude",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(stderr.matches("updated skills").count(), 1, "{stderr}");
    assert!(stderr.contains(&expected), "{stderr}");
    assert!(!before.stdout.windows(9).any(|w| w == b"Installed"));
    assert!(home.path().join(".claude/skills/jidoka/SKILL.md").is_file());
    let text = std::fs::read_to_string(&record).unwrap();
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    let again = jidoka(home.path(), &["run", "inspect", "x"]);
    assert!(!String::from_utf8_lossy(&again.stderr).contains("updated skills"));
    assert_eq!(again.stdout, before.stdout);
}
