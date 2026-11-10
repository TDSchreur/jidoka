//! `jidoka run cleanup` arguments, checked before any server is contacted.
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
fn cleanup_refuses_bad_arguments_with_the_usage() {
    for args in [
        &["run", "cleanup"][..],
        &["run", "cleanup", "--force"],
        &["run", "cleanup", "1", "--force"],
        &["run", "cleanup", "1", "--reinstall"],
        &["run", "cleanup", "1", "--config-dir", "/tmp"],
        &["run", "cleanup", "1", "--config-dir"],
    ] {
        let output = jidoka(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("jidoka run cleanup <run-id>"),
            "{args:?}"
        );
    }
    let output = jidoka(&["run", "cleanup", "1", "--reinstall"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown option: --reinstall"));
    let output = jidoka(&["run", "cleanup", "12x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
