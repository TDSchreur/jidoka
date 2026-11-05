use crate::{
    run::{parse_update, NewRun, Run},
    server::Info,
};
use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

/// Creates a run on the factory server at `server` and returns its id.
pub(crate) fn start_run(
    server: &str,
    title: &str,
    project: &str,
    repository: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    let body = serde_json::to_vec(&NewRun {
        title: title.to_string(),
        project: project.to_string(),
        repository,
    })?;
    let run: Run = serde_json::from_slice(&api_post(server, "/api/runs", &body)?)?;
    Ok(run.id)
}

/// Sends a CLI run update (stage, status and metadata options) to the factory server at `server`.
pub(crate) fn report_run(
    server: &str,
    id: &str,
    stage: &str,
    status: &str,
    options: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    let update = parse_update(stage, status, options)?;
    api_post(
        server,
        &format!("/api/runs/{id}"),
        &serde_json::to_vec(&update)?,
    )?;
    Ok(())
}

/// What answers on a dashboard port.
#[derive(Debug, PartialEq)]
pub(crate) enum Probe {
    /// Nothing is listening.
    Absent,
    /// A Factory dashboard.
    Running(Info),
    /// Something else is using the port.
    Foreign,
}

/// The most a probe reads: far more than an info document, so a chatty listener cannot fill memory.
const PROBE_LIMIT: u64 = 4096;

/// Asks the server at `server` (`host:port`) who it is, through `/api/info`, not the port alone.
/// Anything that does not answer as a Factory dashboard, including a listener that is slow or odd, is `Foreign`.
pub(crate) fn probe_dashboard(server: &str) -> io::Result<Probe> {
    let timeout = Duration::from_secs(2);
    let address = server
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid server address"))?;
    let mut stream = match TcpStream::connect_timeout(&address, timeout) {
        Ok(stream) => stream,
        // Windows reports a closed port as a timeout, not a refusal. Nobody accepted, so nothing runs here;
        // binding the port later reports a real conflict.
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(Probe::Absent)
        }
        Err(error) => return Err(error),
    };
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let sent = write!(
        stream,
        "GET /api/info HTTP/1.1\r\nHost: {server}\r\nConnection: close\r\n\r\n"
    );
    let mut response = Vec::new();
    // A read error keeps what arrived so far; an unfinished answer fails to parse and is foreign.
    let _ = stream.take(PROBE_LIMIT).read_to_end(&mut response);
    if sent.is_err() {
        return Ok(Probe::Foreign);
    }
    Ok(parse_info(&response).map_or(Probe::Foreign, Probe::Running))
}

fn parse_info(response: &[u8]) -> Option<Info> {
    let end = response.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    if !response.starts_with(b"HTTP/1.1 200 ") {
        return None;
    }
    serde_json::from_slice(&response[end..]).ok()
}

/// Reads one run from the factory server at `server`.
pub(crate) fn fetch_run(server: &str, id: &str) -> Result<Run, Box<dyn std::error::Error>> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    Ok(serde_json::from_slice(&api_request(
        server,
        "GET",
        &format!("/api/runs/{id}"),
        &[],
    )?)?)
}

fn api_post(server: &str, route: &str, body: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    api_request(server, "POST", route, body)
}

// Requests name the address they connect to in `Host`, which is the server's own authority.
fn api_request(
    server: &str,
    method: &str,
    route: &str,
    body: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(server).map_err(|e| {
        io::Error::new(e.kind(), format!("cannot connect to factory server at {server}; start it with 'factory dashboard': {e}"))
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    if method == "POST" {
        write!(stream, "POST {route} HTTP/1.1\r\nHost: {server}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
    } else {
        write!(
            stream,
            "{method} {route} HTTP/1.1\r\nHost: {server}\r\nConnection: close\r\n\r\n"
        )?;
    }
    stream.write_all(body)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("invalid server response")?
        + 4;
    let headers = std::str::from_utf8(&response[..end])?;
    if !headers.starts_with("HTTP/1.1 200 ") && !headers.starts_with("HTTP/1.1 201 ") {
        return Err(format!(
            "server rejected request: {}",
            String::from_utf8_lossy(&response[end..])
        )
        .into());
    }
    Ok(response[end..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{server::build_rocket, test_support::TestDir};
    use std::{fs, path::Path};

    /// A real Rocket server on an OS-assigned port, shut down and joined on drop.
    struct TestServer {
        address: String,
        shutdown: rocket::Shutdown,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start(path: &Path) -> Self {
            let rocket = build_rocket(path, 0).unwrap();
            let quiet = rocket.figment().clone().merge(("log_level", "off"));
            let (sender, receiver) = std::sync::mpsc::channel();
            let rocket = rocket
                .configure(quiet)
                .attach(rocket::fairing::AdHoc::on_liftoff(
                    "Report test server",
                    move |rocket| {
                        let _ = sender.send((rocket.config().port, rocket.shutdown()));
                        Box::pin(async {})
                    },
                ));
            let thread = std::thread::spawn(move || {
                if let Err(error) = rocket::execute(rocket.launch()) {
                    eprintln!("test server failed: {error}");
                }
            });
            let (port, shutdown) = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("test server should report its port within 10 seconds");
            Self {
                address: format!("127.0.0.1:{port}"),
                shutdown,
                thread: Some(thread),
            }
        }

        fn runs(&self) -> serde_json::Value {
            let mut stream = TcpStream::connect(&self.address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(
                stream,
                "GET /api/runs HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                self.address
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            let (headers, body) = response.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("HTTP/1.1 200 "), "response: {response}");
            serde_json::from_str(body).unwrap()
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.shutdown.clone().notify();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    #[test]
    fn cli_reports_run_lifecycle_to_real_server_across_restart() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        fs::write(
            &path,
            r#"[{"id":"42","title":"old","project":"/work","stage":"define","status":"running"}]"#,
        )
        .unwrap();
        let server = TestServer::start(&path);
        let id = start_run(&server.address, "Verify slice", "/work", None).unwrap();
        let ticket = "https://github.com/example/repo/issues/7";
        let steps: [(&str, &str, &[&str]); 4] = [
            (
                "build",
                "running",
                &["--activity", "Implementing", "--current-ticket", ticket],
            ),
            (
                "build",
                "blocked",
                &["--blocking-reason", "Need credentials"],
            ),
            (
                "build",
                "running",
                &["--clear-blocking-reason", "--rework", "1"],
            ),
            (
                "finish",
                "done",
                &["--activity", "Opened PR", "--clear-current-ticket"],
            ),
        ];
        let mut snapshots = Vec::new();
        for (stage, status, options) in steps {
            report_run(&server.address, &id, stage, status, options).unwrap();
            snapshots.push(server.runs()[1].clone());
        }
        assert_eq!(snapshots[0]["activity"], "Implementing");
        assert_eq!(snapshots[0]["current_ticket"], ticket);
        assert_eq!(snapshots[1]["status"], "blocked");
        assert_eq!(snapshots[1]["blocking_reason"], "Need credentials");
        assert_eq!(snapshots[1]["activity"], "Implementing");
        assert_eq!(snapshots[2]["status"], "running");
        assert!(snapshots[2].get("blocking_reason").unwrap().is_null());
        assert_eq!(snapshots[2]["current_ticket"], ticket);
        assert_eq!(snapshots[2]["rework"], 1);
        let finished = &snapshots[3];
        assert_eq!(finished["stage"], "finish");
        assert_eq!(finished["status"], "done");
        assert_eq!(finished["activity"], "Opened PR");
        assert_eq!(finished["rework"], 1, "rework count is kept when omitted");
        assert!(finished["current_ticket"].is_null());
        assert!(
            finished["updated_at"].as_u64().unwrap()
                >= snapshots[0]["updated_at"].as_u64().unwrap()
        );

        report_run(
            &server.address,
            "42",
            "check",
            "blocked",
            &["--blocking-reason", "Legacy run"],
        )
        .unwrap();
        drop(server);

        let restarted = TestServer::start(&path);
        let runs = restarted.runs();
        assert_eq!(runs.as_array().unwrap().len(), 2);
        assert_eq!(&runs[1], finished, "finished run survives restart");
        assert_eq!(runs[0]["id"], "42");
        assert_eq!(runs[0]["title"], "old");
        assert_eq!(runs[0]["stage"], "check");
        assert_eq!(runs[0]["blocking_reason"], "Legacy run");
        assert!(runs[0]["updated_at"].is_u64());
    }

    #[test]
    fn cli_reports_rejections_and_unreachable_server() {
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        let missing = report_run(&server.address, "123", "build", "running", &[]).unwrap_err();
        assert!(missing.to_string().contains("Not found"), "{missing}");
        let invalid = report_run(&server.address, "12a", "build", "running", &[]).unwrap_err();
        assert_eq!(invalid.to_string(), "Invalid run id");
        let address = server.address.clone();
        drop(server);
        let unreachable = start_run(&address, "task", "/work", None).unwrap_err();
        assert!(
            unreachable
                .to_string()
                .contains(&format!("cannot connect to factory server at {address}")),
            "{unreachable}"
        );
    }

    #[test]
    fn probe_recognises_a_factory_dashboard_by_its_info() {
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        match probe_dashboard(&server.address).unwrap() {
            Probe::Running(info) => {
                assert_eq!(info.pid, std::process::id());
                assert_eq!(info.version, crate::current_build());
            }
            other => panic!("expected a running dashboard, got {other:?}"),
        }
    }

    #[test]
    fn probe_reports_nothing_listening_and_foreign_listeners() {
        // Nothing listens once the server is gone.
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        let address = server.address.clone();
        drop(server);
        assert_eq!(probe_dashboard(&address).unwrap(), Probe::Absent);

        // A listener that answers something else on the same port is not a dashboard.
        for reply in [
            &b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{\"hello\":1}",
            b"not http at all",
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let thread = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer);
                stream.write_all(reply).unwrap();
            });
            assert_eq!(probe_dashboard(&address).unwrap(), Probe::Foreign);
            thread.join().unwrap();
        }
    }

    #[test]
    fn inspect_reads_back_the_context_a_session_recorded() {
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        let id = start_run(
            &server.address,
            "Continue me",
            "/work/repo.worktrees/x",
            Some("/work/repo".into()),
        )
        .unwrap();
        report_run(
            &server.address,
            &id,
            "build",
            "blocked",
            &[
                "--mode",
                "factory-balanced",
                "--worktree",
                "/work/repo.worktrees/x",
                "--branch",
                "feature/x",
                "--base-commit",
                "abc1234",
                "--spec",
                "https://github.com/o/r/issues/1",
                "--pr",
                "https://github.com/o/r/pull/2",
                "--ticket",
                "https://github.com/o/r/issues/3 Build the thing",
                "--ticket",
                "docs/t.md Document it",
                "--ticket-status",
                "https://github.com/o/r/issues/3=done",
                "--summary",
                "define=Spec and two tickets",
                "--blocking-reason",
                "Need a decision",
            ],
        )
        .unwrap();
        let run = fetch_run(&server.address, &id).unwrap();
        let text = crate::inspect::render(&run);
        for expected in [
            "Stage         build (blocked)",
            "Status reason Need a decision",
            "Mode          factory-balanced",
            "Repository    /work/repo",
            "Worktree      /work/repo.worktrees/x",
            "Branch        feature/x",
            "Base commit   abc1234",
            "Pull request  https://github.com/o/r/pull/2",
            "Tickets       1 of 2 done",
            "[done       ] https://github.com/o/r/issues/3 Build the thing",
            "[pending    ] docs/t.md Document it",
            "define: Spec and two tickets",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(!text.contains("unknown"), "{text}");

        let missing = fetch_run(&server.address, "123").unwrap_err();
        assert!(missing.to_string().contains("Not found"), "{missing}");
        assert_eq!(
            fetch_run(&server.address, "1x").unwrap_err().to_string(),
            "Invalid run id"
        );
        let rejected = report_run(
            &server.address,
            &id,
            "build",
            "running",
            &["--ticket-status", "ghost=done"],
        )
        .unwrap_err();
        assert!(
            rejected.to_string().contains("No ticket ghost"),
            "{rejected}"
        );
    }

    #[test]
    fn inspect_shows_unknown_for_a_run_recorded_before_run_context() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        fs::write(&path, r#"[{"id":"7","title":"old","project":"/work/old","stage":"check","status":"running"}]"#).unwrap();
        let server = TestServer::start(&path);
        let text = crate::inspect::render(&fetch_run(&server.address, "7").unwrap());
        assert!(
            text.contains("Repository    unknown (not recorded)"),
            "{text}"
        );
        assert!(
            text.contains("Branch        unknown (not recorded)"),
            "{text}"
        );
        assert!(text.contains("Started in    /work/old"), "{text}");
        assert!(text.contains("Tickets       none recorded"), "{text}");
    }
}
