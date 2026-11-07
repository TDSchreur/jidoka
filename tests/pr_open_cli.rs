//! `factory pr open` validates arguments before contacting the server.

use std::process::Command;

fn factory(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_factory"))
        .current_dir(dir)
        // No `gh` can be found.
        .env("PATH", "")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn open_refuses_bad_arguments_with_the_usage() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("body.md");
    std::fs::write(&body, "text").unwrap();
    let body = body.to_str().unwrap();
    for args in [
        vec!["pr", "open"],
        vec!["pr", "open", "--json"],
        vec!["pr", "open", "1"],
        vec!["pr", "open", "1", "--title", "T"],
        vec!["pr", "open", "1", "--body-file", body],
        vec![
            "pr",
            "open",
            "1",
            "--title",
            "T",
            "--body-file",
            "/no/such/file",
        ],
        vec![
            "pr",
            "open",
            "1",
            "--title",
            "T",
            "--body-file",
            body,
            "--bogus",
        ],
        vec![
            "pr",
            "open",
            "1",
            "--title",
            "T",
            "--body-file",
            body,
            "--base",
            "-x",
        ],
    ] {
        let output = factory(dir.path(), &args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("factory pr open <run-id>"),
            "{args:?}"
        );
    }
    let output = factory(
        dir.path(),
        &["pr", "open", "12x", "--title", "T", "--body-file", body],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid run id"));
}
