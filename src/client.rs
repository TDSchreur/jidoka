use crate::{
    routing::ModelRouting,
    run::{NewRun, Run, UpdateRun, parse_update},
    server::{CLI_TOKEN_HEADER, DashboardIdentity},
};
use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    time::Duration,
};

/// Creates a run on the Jidoka server at `server` and returns its id.
pub(crate) fn start_run(
    server: &str,
    token_file: &Path,
    title: &str,
    project: &str,
    repository: Option<String>,
    session: Option<String>,
    model_routing: Option<ModelRouting>,
) -> Result<String, Box<dyn std::error::Error>> {
    let body = serde_json::to_vec(&NewRun {
        title: title.to_string(),
        project: project.to_string(),
        repository,
        session,
        model_routing,
    })?;
    let run: Run = serde_json::from_slice(&api_post(server, token_file, "/api/runs", &body)?)?;
    Ok(run.id)
}

/// Sends a CLI run update (stage, status and metadata options) to the Jidoka server at `server`.
pub(crate) fn report_run(
    server: &str,
    token_file: &Path,
    id: &str,
    stage: &str,
    status: &str,
    options: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    send_update(
        server,
        token_file,
        id,
        &parse_update(stage, status, options)?,
    )
}

/// Sends an already built update for run `id` to the Jidoka server at `server`.
pub(crate) fn send_update(
    server: &str,
    token_file: &Path,
    id: &str,
    update: &UpdateRun,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = api_post(
        server,
        token_file,
        &format!("/api/runs/{id}"),
        &serde_json::to_vec(update)?,
    )?;
    for warning in update_warnings(&response) {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}

/// The warnings in an update response; none when it has no list (an older server).
fn update_warnings(response: &[u8]) -> Vec<String> {
    serde_json::from_slice::<serde_json::Value>(response)
        .ok()
        .and_then(|body| serde_json::from_value(body.get("warnings")?.clone()).ok())
        .unwrap_or_default()
}

/// What answers on a dashboard port.
#[derive(Debug, PartialEq)]
pub(crate) enum Probe {
    /// Nothing is listening.
    Absent,
    /// A Jidoka dashboard.
    Running(DashboardIdentity),
    /// Something else is using the port.
    Foreign,
}

/// The most a probe reads: far more than an info document, so a chatty listener cannot fill memory.
const PROBE_LIMIT: u64 = 4096;

/// How long a probe waits to connect: a local dashboard answers at once, so a slower listener is not ours.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long one API call waits on a read or write: the dashboard answers locally, so this only catches a hung server.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Advance can wait through the CI discovery grace and still needs time for host requests.
const ADVANCE_TIMEOUT: Duration = Duration::from_secs(180);

/// Asks the server at `server` (`host:port`) who it is, through `/api/info`, not the port alone.
/// Anything that does not answer as a Jidoka dashboard, including a listener that is slow or odd, is `Foreign`.
pub(crate) fn probe_dashboard(server: &str) -> io::Result<Probe> {
    let address = server
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid server address"))?;
    let mut stream = match TcpStream::connect_timeout(&address, PROBE_TIMEOUT) {
        Ok(stream) => stream,
        // Windows reports a closed port as a timeout, not a refusal. Nobody accepted, so nothing runs here;
        // binding the port later reports a real conflict.
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(Probe::Absent);
        }
        Err(error) => return Err(error),
    };
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
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

fn parse_info(response: &[u8]) -> Option<DashboardIdentity> {
    let end = response.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    if !response.starts_with(b"HTTP/1.1 200 ") {
        return None;
    }
    serde_json::from_slice(&response[end..]).ok()
}

/// Reads one run from the Jidoka server at `server`.
pub(crate) fn fetch_run(server: &str, id: &str) -> Result<Run, Box<dyn std::error::Error>> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    Ok(serde_json::from_slice(&api_request(
        server,
        &format!("/api/runs/{id}"),
        None,
    )?)?)
}

/// Reads every run from the Jidoka server at `server`.
pub(crate) fn fetch_runs(server: &str) -> Result<Vec<Run>, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&api_request(
        server,
        "/api/runs",
        None,
    )?)?)
}

pub(crate) fn api_post(
    server: &str,
    token_file: &Path,
    route: &str,
    body: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Read on every call, so a restarted dashboard needs no manual step. The token is never put in an error.
    let token = crate::token::read_file(token_file)
        .ok_or("no CLI token for the Jidoka server; start or restart 'jidoka dashboard'")?;
    api_request(server, route, Some((&token, body)))
}

// Requests name the address they connect to in `Host`, which is the server's own authority.
// `post` is the CLI token and body of a POST; without it the request is a bodiless GET.
fn api_request(
    server: &str,
    route: &str,
    post: Option<(&str, &[u8])>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(server).map_err(|e| {
        io::Error::new(e.kind(), format!("cannot connect to the Jidoka server at {server}; start it with 'jidoka dashboard': {e}"))
    })?;
    let timeout = if route.ends_with("/advance") {
        ADVANCE_TIMEOUT
    } else {
        REQUEST_TIMEOUT
    };
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    match post {
        Some((cli_token, body)) => {
            write!(
                stream,
                "POST {route} HTTP/1.1\r\nHost: {server}\r\nContent-Type: application/json\r\n{CLI_TOKEN_HEADER}: {cli_token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
        }
        None => write!(
            stream,
            "GET {route} HTTP/1.1\r\nHost: {server}\r\nConnection: close\r\n\r\n"
        )?,
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("invalid server response")?
        + 4;
    let headers = std::str::from_utf8(&response[..end])?;
    if headers.starts_with("HTTP/1.1 401 ") {
        return Err("the Jidoka server rejected the CLI token; restart 'jidoka dashboard'".into());
    }
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

    /// An address nobody listens on: a port the tests' own servers can never be assigned.
    const NOTHING_LISTENS: &str = "127.0.0.1:1";

    /// A real Rocket server on an OS-assigned port, shut down and joined on drop.
    struct TestServer {
        address: String,
        token_file: std::path::PathBuf,
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
                token_file: crate::server::token_path(path, port),
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
    fn gate_results_and_advance_survive_server_restart() {
        let dir = TestDir::new();
        let repo = dir.0.path().join("repo");
        fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "Initial",
            ],
        ] {
            let output = std::process::Command::new("git")
                .current_dir(&repo)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", dir.0.path().join("no-config"))
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .args(&args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let path = dir.0.path().join("runs.json");
        let server = TestServer::start(&path);
        let id = start_run(
            &server.address,
            &server.token_file,
            "Review",
            "/work",
            None,
            None,
            None,
        )
        .unwrap();
        report_run(
            &server.address,
            &server.token_file,
            &id,
            "check",
            "running",
            &[
                "--mode",
                "jidoka-balanced",
                "--worktree",
                repo.to_str().unwrap(),
            ],
        )
        .unwrap();
        for gate in ["checks-recorded", "no-actionable-findings", "verified"] {
            let body = serde_json::json!({"gate": gate, "passed":true, "evidence":"reviewed"});
            api_post(
                &server.address,
                &server.token_file,
                &format!("/api/runs/{id}/gate"),
                &serde_json::to_vec(&body).unwrap(),
            )
            .unwrap();
        }
        api_post(
            &server.address,
            &server.token_file,
            &format!("/api/runs/{id}/advance"),
            b"{}",
        )
        .unwrap();
        drop(server);
        let restarted = TestServer::start(&path);
        let run = fetch_run(&restarted.address, &id).unwrap();
        assert_eq!(run.stage, crate::run::Stage::Finish);
        assert_eq!(run.gates.len(), 3);
        assert_eq!(run.gates[0].evidence.as_deref(), Some("reviewed"));
    }

    #[test]
    fn cli_reports_run_lifecycle_to_real_server_across_restart() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        fs::write(
            &path,
            r#"[{"id":"42","title":"old","gates":[],"project":"/work","stage":"define","status":"running"}]"#,
        )
        .unwrap();
        let server = TestServer::start(&path);
        let id = start_run(
            &server.address,
            &server.token_file,
            "Verify slice",
            "/work",
            None,
            None,
            None,
        )
        .unwrap();
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
                &[
                    "--summary",
                    "finish=Shipped",
                    "--activity",
                    "Opened PR",
                    "--clear-current-ticket",
                ],
            ),
        ];
        let mut snapshots = Vec::new();
        for (stage, status, options) in steps {
            report_run(
                &server.address,
                &server.token_file,
                &id,
                stage,
                status,
                options,
            )
            .unwrap();
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
            &server.token_file,
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
        let missing = report_run(
            &server.address,
            &server.token_file,
            "123",
            "build",
            "running",
            &[],
        )
        .unwrap_err();
        assert!(missing.to_string().contains("Not found"), "{missing}");
        let invalid = report_run(
            &server.address,
            &server.token_file,
            "12a",
            "build",
            "running",
            &[],
        )
        .unwrap_err();
        assert_eq!(invalid.to_string(), "Invalid run id");
        // Nothing listens on port 1. A port that was just freed could be taken by another test's server.
        let address = NOTHING_LISTENS.to_string();
        let unreachable = start_run(
            &address,
            &server.token_file,
            "task",
            "/work",
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            unreachable
                .to_string()
                .contains(&format!("cannot connect to the Jidoka server at {address}")),
            "{unreachable}"
        );
    }

    /// The server's current CLI token, as the file holds it.
    fn token_of(server: &TestServer) -> String {
        fs::read_to_string(&server.token_file)
            .unwrap()
            .trim()
            .to_string()
    }

    #[test]
    fn cli_keeps_working_after_the_dashboard_restarts_with_a_new_token() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let first = TestServer::start(&path);
        let id = start_run(
            &first.address,
            &first.token_file,
            "t",
            "/w",
            None,
            None,
            None,
        )
        .unwrap();
        let old = token_of(&first);
        drop(first);
        let second = TestServer::start(&path);
        assert_ne!(token_of(&second), old, "a restart issues a new token");
        report_run(
            &second.address,
            &second.token_file,
            &id,
            "build",
            "running",
            &[],
        )
        .unwrap();
        assert_eq!(second.runs()[0]["stage"], "build");
    }

    #[test]
    fn missing_or_unreadable_token_file_points_to_the_dashboard_without_the_token() {
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        let token = token_of(&server);
        fs::remove_file(&server.token_file).unwrap();
        let missing = start_run(
            &server.address,
            &server.token_file,
            "t",
            "/w",
            None,
            None,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(missing.contains("jidoka dashboard"), "{missing}");
        assert!(!missing.contains(&token), "{missing}");
        // A directory in place of the file is unreadable.
        fs::create_dir(&server.token_file).unwrap();
        let unreadable = report_run(
            &server.address,
            &server.token_file,
            "1",
            "build",
            "running",
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(unreadable.contains("jidoka dashboard"), "{unreadable}");
        assert!(server.runs().as_array().unwrap().is_empty());
    }

    #[test]
    fn rejected_token_hints_at_restarting_the_dashboard_without_leaking_tokens() {
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        let real = token_of(&server);
        let stale = dir.0.path().join("stale-token");
        fs::write(&stale, "stale-token-value\n").unwrap();
        let error = start_run(&server.address, &stale, "t", "/w", None, None, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("restart 'jidoka dashboard'"), "{error}");
        assert!(
            !error.contains("stale-token-value") && !error.contains(&real),
            "{error}"
        );
        let error = report_run(&server.address, &stale, "1", "build", "running", &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("restart 'jidoka dashboard'"), "{error}");
    }

    #[test]
    fn probe_recognises_a_jidoka_dashboard_by_its_info() {
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

    /// A policy file with an entry for each agent; the models tell the entries apart.
    fn both_agents(claude_model: &str, codex_model: &str) -> String {
        format!(
            r#"{{"agents":{{
                "claude":{{"stages":{{"check":{{"model":"{claude_model}"}}}}}},
                "codex":{{"stages":{{"check":{{"model":"{codex_model}","effort":"high"}}}}}}}}}}"#
        )
    }

    #[test]
    fn a_run_keeps_the_selection_it_started_with_when_the_policy_file_changes() {
        use crate::{
            routing::{Host, RoutingAgent},
            run::Stage,
        };
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        let policy = dir.0.path().join("model-routing.json");
        fs::write(&policy, both_agents("opus", "gpt-6-astra")).unwrap();
        let codex = crate::routing::load(&policy, Some(Host::Codex))
            .unwrap()
            .selection;
        let id = start_run(
            &server.address,
            &server.token_file,
            "Routed",
            "/work",
            None,
            None,
            codex,
        )
        .unwrap();
        fs::write(&policy, both_agents("opus", "gpt-6-luna")).unwrap();
        report_run(
            &server.address,
            &server.token_file,
            &id,
            "build",
            "running",
            &["--activity", "go"],
        )
        .unwrap();
        // The run holds only the Codex entry as it was at the start, and a continued session sees the same.
        let run = fetch_run(&server.address, &id).unwrap();
        assert!(crate::inspect::render(&run).contains("Model routing codex"));
        let routing = run.model_routing.unwrap();
        assert_eq!(routing.agent, RoutingAgent::Codex);
        assert_eq!(routing.stages[&Stage::Check].model, "gpt-6-astra");
        assert_eq!(routing.stages.len(), 1);
        // A per-run override keeps working on the recorded selection.
        report_run(
            &server.address,
            &server.token_file,
            &id,
            "check",
            "running",
            &["--model-routing", "check=opus:low"],
        )
        .unwrap();
        let run = fetch_run(&server.address, &id).unwrap();
        assert!(
            run.model_routing
                .unwrap()
                .overrides
                .contains_key(&Stage::Check)
        );
        // A run started now reads the new file, and a Claude run gets the Claude entry, never Codex's.
        let codex = crate::routing::load(&policy, Some(Host::Codex))
            .unwrap()
            .selection;
        let next = start_run(
            &server.address,
            &server.token_file,
            "Next",
            "/work",
            None,
            None,
            codex,
        )
        .unwrap();
        let run = fetch_run(&server.address, &next).unwrap();
        assert_eq!(
            run.model_routing.unwrap().stages[&Stage::Check].model,
            "gpt-6-luna"
        );
        let claude = crate::routing::load(&policy, Some(Host::Claude))
            .unwrap()
            .selection;
        let third = start_run(
            &server.address,
            &server.token_file,
            "Claude",
            "/work",
            None,
            None,
            claude,
        )
        .unwrap();
        let routing = fetch_run(&server.address, &third)
            .unwrap()
            .model_routing
            .unwrap();
        assert_eq!(routing.agent, RoutingAgent::Claude);
        assert_eq!(routing.stages[&Stage::Check].model, "opus");
    }

    #[test]
    fn inspect_reads_back_the_context_a_session_recorded() {
        let dir = TestDir::new();
        let server = TestServer::start(&dir.0.path().join("runs.json"));
        let id = start_run(
            &server.address,
            &server.token_file,
            "Continue me",
            "/work/repo.worktrees/x",
            Some("/work/repo".into()),
            None,
            None,
        )
        .unwrap();
        report_run(
            &server.address,
            &server.token_file,
            &id,
            "build",
            "blocked",
            &[
                "--mode",
                "jidoka-balanced",
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
            "Mode          jidoka-balanced",
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
            &server.token_file,
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
}
