//! `jidoka run sync` arguments, checked before any server is contacted.
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

use std::process::Command;

fn jidoka(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jidoka"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn sync_refuses_bad_arguments_with_the_usage() {
    for args in [
        &["run", "sync"][..],
        &["run", "sync", "--json"],
        &["run", "sync", "1", "--bogus"],
        &["run", "sync", "1", "--json", "--json"],
    ] {
        let output = jidoka(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("jidoka run sync <run-id>"),
            "{args:?}"
        );
    }
    let output = jidoka(&["run", "sync", "12x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
