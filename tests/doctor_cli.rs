//! `factory doctor` as a user runs it: exit status, JSON and argument errors.

use std::process::{Command, Output};

fn factory(args: &[&str], dir: &std::path::Path, home: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("PATH", "")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_CODE_SUBAGENT_MODEL_FORCE")
        .env_remove("CLAUDE_CODE_SUBAGENT_MODEL")
        .env_remove("CLAUDE_CODE_EFFORT_LEVEL")
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("APPDATA", home.join(".config"))
        .output()
        .unwrap()
}

#[test]
fn doctor_fails_with_a_json_report_when_nothing_is_set_up() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(&["doctor", "--json"], dir.path(), home.path());
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["agents"].as_array().unwrap().len(), 3);
}

#[test]
fn doctor_with_an_agent_reports_only_that_agent() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(
        &["doctor", "--agent", "codex", "--json"],
        dir.path(),
        home.path(),
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["agents"].as_array().unwrap().len(), 1);
    assert_eq!(report["agents"][0]["id"], "codex");
}

#[test]
fn doctor_rejects_unknown_arguments_without_a_report() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(&["doctor", "--agent", "gemini"], dir.path(), home.path());
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown agent"));
}

#[test]
fn help_lists_doctor() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(&["--help"], dir.path(), home.path());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("factory doctor [--agent <codex|claude|copilot>] [--json]"));
}

/// The folder holding `git`, so a run can use git and still see no agent on `PATH`.
fn git_folder() -> std::path::PathBuf {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .find(|dir| dir.join(name).is_file())
        .expect("git is on PATH")
}

#[test]
fn doctor_reads_the_repository_it_runs_in() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-q"][..],
        &["remote", "add", "origin", "https://example.com/repo.git"],
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(["doctor", "--json"])
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("PATH", git_folder())
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_CODE_SUBAGENT_MODEL_FORCE")
        .env_remove("CLAUDE_CODE_SUBAGENT_MODEL")
        .env_remove("CLAUDE_CODE_EFFORT_LEVEL")
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("APPDATA", home.path().join(".config"))
        .output()
        .unwrap();
    // No agent and no setup docs: not ready, but the repository checks see the real repository.
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let status = |id: &str| {
        report["project"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status("git"), "pass");
    assert_eq!(status("origin"), "pass");
    assert_eq!(status("base-branch"), "pass");
    assert_eq!(status("setup-docs"), "missing");
}

#[test]
fn doctor_creates_the_model_routing_policy_once_and_keeps_json_output_valid() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let policy = home.path().join(".config/tds-factory/model-routing.json");
    let run = || {
        factory(
            &[
                "doctor",
                "--agent",
                "claude",
                "--create-model-routing",
                "--json",
            ],
            dir.path(),
            home.path(),
        )
    };

    let first = run();
    // Stdout is still one JSON report, and the note about the file is on stderr.
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["model_routing"][0]["id"], "policy");
    assert_eq!(report["model_routing"][0]["status"], "pass");
    let note = String::from_utf8_lossy(&first.stderr);
    assert!(
        note.contains("Created") && note.contains("model-routing.json"),
        "{note}"
    );
    let text = std::fs::read_to_string(&policy).unwrap();
    assert!(
        text.contains("\"claude\"") && !text.contains("\"codex\""),
        "{text}"
    );

    // A second run, even after an edit, leaves the file as it is.
    std::fs::write(&policy, "{\"agents\":{\"codex\":{\"stages\":{}}}}").unwrap();
    let second = run();
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
    assert!(std::fs::read_to_string(&policy).unwrap().contains("codex"));
}

#[test]
fn doctor_creates_the_codex_starter_too_and_reports_when_it_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(
        &["doctor", "--agent", "codex", "--create-model-routing"],
        dir.path(),
        home.path(),
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("Created") && text.contains("starter policy for codex"),
        "{text}"
    );
    assert!(
        text.contains("pass     policy:") && text.contains("configures codex"),
        "{text}"
    );
    let written =
        std::fs::read_to_string(home.path().join(".config/tds-factory/model-routing.json"))
            .unwrap();
    assert!(written.contains("gpt-6.1-terra"), "{written}");

    // A file where the folder should be: doctor stops with the reason and a way out.
    let blocked = tempfile::tempdir().unwrap();
    std::fs::write(blocked.path().join(".config"), "a file").unwrap();
    let output = factory(
        &[
            "doctor",
            "--agent",
            "claude",
            "--create-model-routing",
            "--json",
        ],
        dir.path(),
        blocked.path(),
    );
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "no report is printed after a failed write"
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("cannot create") && error.contains("create the file yourself"),
        "{error}"
    );
}

#[test]
fn doctor_creates_one_file_for_both_agents_and_checks_each_of_them() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(
        &["doctor", "--create-model-routing", "--json"],
        dir.path(),
        home.path(),
    );
    let note = String::from_utf8_lossy(&output.stderr);
    assert!(note.contains("claude and codex"), "{note}");
    let written =
        std::fs::read_to_string(home.path().join(".config/tds-factory/model-routing.json"))
            .unwrap();
    assert!(
        written.contains("\"claude\"") && written.contains("\"codex\""),
        "{written}"
    );
    // The next report checks both entries, each tagged with its agent.
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let agents: std::collections::BTreeSet<&str> = report["model_routing"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|check| check["agent"].as_str())
        .collect();
    assert_eq!(agents, ["claude", "codex"].into());
    // Asking for one agent checks only its entry.
    let output = factory(
        &["doctor", "--agent", "codex", "--json"],
        dir.path(),
        home.path(),
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["model_routing"]
        .as_array()
        .unwrap()
        .iter()
        .all(|check| check["agent"].is_null() || check["agent"] == "codex"));
}

#[test]
fn doctor_will_not_create_a_policy_for_copilot() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = factory(
        &["doctor", "--create-model-routing", "--agent", "copilot"],
        dir.path(),
        home.path(),
    );
    assert!(!output.status.success());
    assert!(
        !home.path().join(".config").exists(),
        "something was written"
    );
}

const OLD_MULTI: &str =
    r#"{"agents":{"claude":{"roles":{"fast":{"model":"haiku"}},"tasks":{"build":"fast"}}}}"#;
const OLD_SINGLE: &str =
    r#"{"agent":"claude","roles":{"fast":{"model":"haiku"}},"tasks":{"build":"fast"}}"#;

fn create_over(existing: &str) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let policy = home.path().join(".config/tds-factory/model-routing.json");
    std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
    std::fs::write(&policy, existing).unwrap();
    let output = factory(
        &["doctor", "--create-model-routing"],
        dir.path(),
        home.path(),
    );
    (
        std::fs::read_to_string(&policy).unwrap(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn doctor_replaces_an_old_format_policy_in_either_form_and_says_so() {
    for old in [OLD_MULTI, OLD_SINGLE] {
        let (text, stdout) = create_over(old);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value["agents"]["claude"]["stages"]["build"].is_object());
        assert!(value["agents"]["codex"]["stages"]["build"].is_object());
        assert!(
            stdout.contains("Replaced old-format model routing policy")
                && stdout.contains("model-routing.json"),
            "{stdout}"
        );
        assert!(stdout.contains("pass     policy:"), "{stdout}");
    }
}

#[test]
fn doctor_leaves_a_current_format_or_unrecognised_policy_alone() {
    for existing in [
        r#"{"agents":{"codex":{"stages":{"build":{"model":"x"}}}}}"#,
        "not json at all",
        r#"{"something":"else"}"#,
    ] {
        let (text, stdout) = create_over(existing);
        assert_eq!(text, existing);
        assert!(!stdout.contains("Replaced"), "{stdout}");
        assert!(stdout.contains("already exists"), "{stdout}");
    }
}
