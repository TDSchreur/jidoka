use crate::{
    run::{parse_update, NewRun, Run},
    server::ADDRESS,
};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    time::Duration,
};

/// Creates a run on the factory server at `server` and returns its id.
pub(crate) fn start_run(
    server: &str,
    title: &str,
    project: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let body = serde_json::to_vec(&NewRun {
        title: title.to_string(),
        project: project.to_string(),
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

// Requests always name the server's trusted authority in `Host`, whichever address they connect to.
fn api_post(server: &str, route: &str, body: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(server).map_err(|e| {
        io::Error::new(e.kind(), format!("cannot connect to factory server at {server}; start it with 'factory dashboard': {e}"))
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(stream, "POST {route} HTTP/1.1\r\nHost: {ADDRESS}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
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
                "GET /api/runs HTTP/1.1\r\nHost: {ADDRESS}\r\nConnection: close\r\n\r\n"
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
        let id = start_run(&server.address, "Verify slice", "/work").unwrap();
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
        let unreachable = start_run(&address, "task", "/work").unwrap_err();
        assert!(
            unreachable
                .to_string()
                .contains(&format!("cannot connect to factory server at {address}")),
            "{unreachable}"
        );
    }
}
