//! `jidoka run check` arguments, checked before any server is contacted.
#![allow(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

mod common;

fn jidoka(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    common::jidoka_in(home.path()).args(args).output().unwrap()
}

#[test]
fn check_refuses_bad_arguments_with_the_usage() {
    for args in [
        &["run", "check"][..],
        &["run", "check", "--json"],
        &["run", "check", "1", "--bogus"],
        &["run", "check", "1", "--json", "--json"],
    ] {
        let output = jidoka(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("jidoka run check <run-id>"),
            "{args:?}"
        );
    }
    let output = jidoka(&["run", "check", "12x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
