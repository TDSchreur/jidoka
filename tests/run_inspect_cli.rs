//! `factory run inspect` arguments, checked before any server is contacted.

use std::process::Command;

fn factory(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn inspect_refuses_unknown_options_and_bad_ids() {
    for args in [
        &["run", "inspect", "1", "--bogus"][..],
        &["run", "inspect", "1", "--json", "--json"],
        &["run", "inspect"],
    ] {
        let output = factory(args);
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("factory run inspect <id> [--json]"),
            "{args:?}"
        );
    }
    let output = factory(&["run", "inspect", "12x"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
