//! `jidoka run begin` arguments, checked before any run is created.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

mod common;

fn jidoka_in(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    // The user's own settings must not reach the test.
    common::jidoka_in(dir)
        .current_dir(dir)
        .env("HOME", dir)
        .env("USERPROFILE", dir)
        .env("XDG_CONFIG_HOME", dir.join(".config"))
        .env("APPDATA", dir.join(".config"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn begin_refuses_bad_arguments_with_the_usage() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["run", "begin"][..],
        &["run", "begin", "--json"],
        &["run", "begin", "Title", "--bogus"],
        &["run", "begin", "Title", "--mode", "turbo"],
        &["run", "begin", "Title", "--base"],
        &["run", "begin", "Title", "--base", "-x"],
        &["run", "begin", "Title", "--json", "--json"],
        &["run", "begin", "Title", "--agent", "nobody"],
        &["run", "begin", "--run"],
        &["run", "begin", "--run", "12x"],
        &["run", "begin", "--run", "5", "--bogus"],
    ] {
        let output = jidoka_in(dir.path(), args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("jidoka run begin <title>"),
            "{args:?}"
        );
    }
}

const TWO_AGENT_POLICY: &str = r#"{"enabled": true, "agents": {
  "claude": {"stages": {"check": {"model": "opus", "effort": "high"}}},
  "codex": {"stages": {"check": {"model": "gpt-6-astra"}}}}}"#;

#[test]
fn a_new_run_needs_the_agent_when_the_policy_has_several_but_a_resume_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".config/jidoka");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("model-routing.json"), TWO_AGENT_POLICY).unwrap();

    let new = jidoka_in(dir.path(), &["run", "begin", "Title"]);
    assert_eq!(new.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&new.stderr).contains("--agent"));

    // Outside a repository the resume stops at the repository, never at the policy.
    let resume = jidoka_in(dir.path(), &["run", "begin", "--run", "5"]);
    assert_eq!(resume.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&resume.stderr);
    assert!(
        stderr.contains("git repository") && !stderr.contains("--agent"),
        "{stderr}"
    );
}
