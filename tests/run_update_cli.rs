//! `factory run update` prints the server's warnings on stderr and still succeeds.

use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

#[test]
#[cfg(debug_assertions)]
fn update_prints_each_warning_on_stderr_and_exits_zero() {
    let data = tempfile::tempdir().unwrap();
    let token_dir = data.path().join("tds-factory");
    std::fs::create_dir_all(&token_dir).unwrap();
    std::fs::write(
        token_dir.join("session-token"),
        "0123456789abcdef".repeat(4),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Read the whole request (headers, then the body they announce) before answering.
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&request).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
            assert!(read > 0, "the client closed before sending a full request");
        }
        let body = r#"{"id":"1","warnings":["first warning","second warning"]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let output = Command::new(env!("CARGO_BIN_EXE_factory"))
        .args(["run", "update", "1", "finish", "done"])
        .env("FACTORY_TEST_ADDRESS", &address)
        .env("XDG_DATA_HOME", data.path())
        .env_remove("CLAUDE_SESSION_ID")
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Updated run 1\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Warning: first warning\n"), "{stderr}");
    assert!(stderr.contains("Warning: second warning\n"), "{stderr}");
}
