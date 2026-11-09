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

#[cfg(debug_assertions)]
fn inspect_with_fake_server(data: &std::path::Path, args: &[&str]) -> std::process::Output {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "the client closed before sending a request");
            request.extend_from_slice(&chunk[..read]);
        }
        let body =
            r#"{"id":"7","title":"Plan run","project":"p","stage":"build","status":"running"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let output = Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(args)
        .env("FACTORY_TEST_ADDRESS", &address)
        .env("XDG_DATA_HOME", data)
        .env("LOCALAPPDATA", data)
        .output()
        .unwrap();
    server.join().unwrap();
    output
}

#[test]
#[cfg(debug_assertions)]
fn inspect_shows_the_plan_path_in_text_and_creates_its_folder() {
    let data = tempfile::tempdir().unwrap();
    let output = inspect_with_fake_server(data.path(), &["run", "inspect", "7"]);
    assert!(output.status.success(), "{output:?}");
    let plan = data.path().join("tds-factory").join("plans").join("7.md");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("Plan          {}\n", plan.display())),
        "{stdout}"
    );
    assert!(plan.parent().unwrap().is_dir());
}

#[test]
#[cfg(debug_assertions)]
fn inspect_shows_the_plan_path_in_json() {
    let data = tempfile::tempdir().unwrap();
    let output = inspect_with_fake_server(data.path(), &["run", "inspect", "7", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["plan"],
        data.path()
            .join("tds-factory")
            .join("plans")
            .join("7.md")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(json["id"], "7");
}
