//! `factory run check` arguments, checked before any server is contacted.

use std::process::Command;

fn factory(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn check_refuses_bad_arguments_with_the_usage() {
    for args in [
        &["run", "check"][..],
        &["run", "check", "--json"],
        &["run", "check", "1", "--bogus"],
        &["run", "check", "1", "--json", "--json"],
    ] {
        let output = factory(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("factory run check <run-id>"),
            "{args:?}"
        );
    }
    let output = factory(&["run", "check", "12x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
