//! `factory pr wait` arguments and a missing `gh`, checked before any server is contacted.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

use std::process::Command;

fn factory(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_factory"))
        // No `gh` can be found.
        .env("PATH", "")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn wait_refuses_unknown_options_and_bad_ids() {
    for args in [
        &["pr", "wait"][..],
        &["pr", "wait", "--json"],
        &["pr", "wait", "1", "--bogus"],
        &["pr", "wait", "1", "--json", "--json"],
    ] {
        let output = factory(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("factory pr wait <run-id>"),
            "{args:?}"
        );
    }
    let output = factory(&["pr", "wait", "12x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}

#[test]
fn a_bad_timeout_is_refused_before_anything_else_is_looked_at() {
    for value in ["0", "abc", "25h", "5x"] {
        let output = factory(&["pr", "wait", "1", "--timeout", value]);
        assert_eq!(output.status.code(), Some(1), "{value}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Invalid --timeout"), "{value}: {stderr}");
        // Not the missing `gh`: the arguments are checked first.
        assert!(!stderr.contains("GitHub CLI"), "{value}: {stderr}");
    }
    let output = factory(&["pr", "wait", "1", "--timeout"]);
    assert_eq!(output.status.code(), Some(1));
}
