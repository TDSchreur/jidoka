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

#[test]
fn stats_needs_a_run_id_or_all() {
    for args in [
        &["run", "stats"][..],
        &["run", "stats", "--bogus"],
        &["run", "stats", "1", "--all"],
    ] {
        let output = factory(args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("factory run stats <id> | --all"),
            "{args:?}"
        );
    }
    let output = factory(&["run", "stats", "12x"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}

#[test]
fn requests_refuses_unknown_options_and_bad_ids() {
    for args in [
        &["run", "requests"][..],
        &["run", "requests", "1", "--bogus"],
        &["run", "requests", "--json"],
        &["run", "requests", "--all", "--bogus"],
        &["run", "requests", "--all", "--exclude"],
        &["run", "requests", "--all", "--exclude", "12x"],
        &["run", "requests", "--all", "--json", "--json"],
    ] {
        let output = factory(args);
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("factory run requests <id> | --all"),
            "{args:?}"
        );
    }
    let output = factory(&["run", "requests", "12x"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
