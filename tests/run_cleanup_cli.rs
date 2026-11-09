//! `factory run cleanup` arguments, checked before any server is contacted.

use std::process::Command;

fn factory(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_factory"))
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
        &["run", "cleanup", "1", "--reinstall", "--force"],
        &["run", "cleanup", "1", "--config-dir"],
    ] {
        let output = factory(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("factory run cleanup <run-id>"),
            "{args:?}"
        );
    }
    let output = factory(&["run", "cleanup", "12x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
