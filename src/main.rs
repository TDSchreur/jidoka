use serde::{Deserialize, Serialize};
mod install;
use std::{
    env, fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ADDRESS: &str = "127.0.0.1:4311";
const DASHBOARD: &str = include_str!("dashboard.html");
const HELP: &str = "TDS Factory\n\nUsage:\n  factory2 install --agent <codex|claude|copilot>\n  factory2 dashboard\n  factory2 run start <title>\n  factory2 run update <id> <define|build|check|finish> <running|blocked|done>\n  factory2 --help | --version";
const LIMIT: usize = 65536;

#[derive(Clone, Serialize, Deserialize)]
struct Run {
    id: String,
    title: String,
    project: String,
    stage: Stage,
    status: Status,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Stage {
    Define,
    Build,
    Check,
    Finish,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Running,
    Blocked,
    Done,
}

#[derive(Deserialize, Serialize)]
struct NewRun {
    title: String,
    project: String,
}
#[derive(Deserialize, Serialize)]
struct UpdateRun {
    stage: Stage,
    status: Status,
}

fn main() -> ExitCode {
    if let Err(error) = run_cli() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--help" | "-h"] => println!("{HELP}"),
        ["--version" | "-V"] => println!("factory2 {}", env!("CARGO_PKG_VERSION")),
        ["install", "--agent", agent] => install::install(agent)?,
        ["dashboard"] => serve(&data_path()?)?,
        ["run", "start", title] => {
            let body = serde_json::to_vec(&NewRun {
                title: title.to_string(),
                project: env::current_dir()?.display().to_string(),
            })?;
            let response = api_post("/api/runs", &body)?;
            let run: Run = serde_json::from_slice(&response)?;
            println!("{}", run.id);
        }
        ["run", "update", id, stage, status] => {
            if !id.bytes().all(|b| b.is_ascii_digit()) || id.is_empty() {
                return Err("Invalid run id".into());
            }
            let stage: Stage =
                serde_json::from_value(serde_json::Value::String(stage.to_string()))?;
            let status: Status =
                serde_json::from_value(serde_json::Value::String(status.to_string()))?;
            api_post(
                &format!("/api/runs/{id}"),
                &serde_json::to_vec(&UpdateRun { stage, status })?,
            )?;
            println!("Updated run {id}");
        }
        _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
    }
    Ok(())
}

fn data_path() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let base = base.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot determine the user data directory",
        )
    })?;
    Ok(base.join("tds-factory2/runs.json"))
}

fn load_runs(path: &Path) -> io::Result<Vec<Run>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

fn save_runs(path: &Path, runs: &[Run]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("run storage has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec(runs)?)?;
    fs::rename(temp, path)
}

fn serve(path: &Path) -> io::Result<()> {
    let listener = TcpListener::bind(ADDRESS)
        .map_err(|e| io::Error::new(e.kind(), format!("could not listen on {ADDRESS}: {e}")))?;
    println!("Dashboard: http://{ADDRESS} (press Ctrl+C to stop)");
    for connection in listener.incoming() {
        match connection.and_then(|stream| respond(stream, path)) {
            Ok(()) => (),
            Err(error) => eprintln!("Dashboard request error: {error}"),
        }
    }
    Ok(())
}

fn respond(mut stream: TcpStream, path: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut data = Vec::new();
    let mut buf = [0; 4096];
    let header_end = loop {
        let n = read_before(&mut stream, &mut buf, deadline)?;
        if n == 0 {
            return Ok(());
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() > LIMIT {
            return write_response(&mut stream, 413, "text/plain", b"Request too large");
        }
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let headers = match std::str::from_utf8(&data[..header_end]) {
        Ok(h) => h,
        Err(_) => return write_response(&mut stream, 400, "text/plain", b"Bad request"),
    };
    let mut lines = headers.split("\r\n");
    let request: Vec<_> = lines.next().unwrap_or("").split_whitespace().collect();
    if request.len() != 3 || !matches!(request[2], "HTTP/1.0" | "HTTP/1.1") {
        return write_response(&mut stream, 400, "text/plain", b"Bad request");
    }
    let method = request[0].to_owned();
    let route = request[1].to_owned();
    let mut length = None;
    let mut json = false;
    let mut origin = false;
    let mut host_ok = false;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return write_response(&mut stream, 400, "text/plain", b"Bad header");
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return write_response(&mut stream, 400, "text/plain", b"Duplicate length");
            }
            length = value.parse::<usize>().ok();
            if length.is_none() {
                return write_response(&mut stream, 400, "text/plain", b"Invalid length");
            }
        } else if name.eq_ignore_ascii_case("content-type") {
            json = value.eq_ignore_ascii_case("application/json");
        } else if name.eq_ignore_ascii_case("origin") {
            origin = true;
        } else if name.eq_ignore_ascii_case("host") {
            host_ok = matches!(value, "127.0.0.1:4311" | "localhost:4311");
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return write_response(&mut stream, 400, "text/plain", b"Unsupported encoding");
        }
    }
    if !host_ok {
        return write_response(&mut stream, 400, "text/plain", b"Invalid host");
    }
    let length = length.unwrap_or(0);
    if length > LIMIT || header_end + length > LIMIT {
        return write_response(&mut stream, 413, "text/plain", b"Request too large");
    }
    while data.len() < header_end + length {
        let n = read_before(&mut stream, &mut buf, deadline)?;
        if n == 0 {
            return write_response(&mut stream, 400, "text/plain", b"Incomplete body");
        }
        data.extend_from_slice(&buf[..n]);
    }
    if method == "POST" && (!json || origin) {
        return write_response(&mut stream, 403, "text/plain", b"Forbidden");
    }
    let (status, kind, body) = handle(
        &method,
        &route,
        &data[header_end..header_end + length],
        path,
    );
    write_response(&mut stream, status, kind, &body)
}

fn read_before(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> io::Result<usize> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request deadline exceeded"))?;
    stream.set_read_timeout(Some(remaining))?;
    stream.read(buf)
}

fn handle(method: &str, route: &str, body: &[u8], path: &Path) -> (u16, &'static str, Vec<u8>) {
    let result = (|| -> Result<(u16, &'static str, Vec<u8>), Box<dyn std::error::Error>> {
        if method == "GET" && route == "/" {
            return Ok((
                200,
                "text/html; charset=utf-8",
                DASHBOARD.as_bytes().to_vec(),
            ));
        }
        if method == "GET" && route == "/api/runs" {
            return Ok((
                200,
                "application/json",
                serde_json::to_vec(&load_runs(path)?)?,
            ));
        }
        if method == "POST" && route == "/api/runs" {
            let new: NewRun = match serde_json::from_slice(body) {
                Ok(value) => value,
                Err(_) => return Ok((400, "text/plain", b"Invalid run".to_vec())),
            };
            if new.title.trim().is_empty()
                || new.project.trim().is_empty()
                || new.title.len() > 500
                || new.project.len() > 4096
            {
                return Ok((400, "text/plain", b"Invalid run".to_vec()));
            }
            let mut runs = load_runs(path)?;
            let mut id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            while runs.iter().any(|r| r.id == id.to_string()) {
                id += 1;
            }
            let run = Run {
                id: id.to_string(),
                title: new.title,
                project: new.project,
                stage: Stage::Define,
                status: Status::Running,
            };
            runs.push(run.clone());
            save_runs(path, &runs)?;
            return Ok((201, "application/json", serde_json::to_vec(&run)?));
        }
        if method == "POST" && route.starts_with("/api/runs/") {
            let id = route.trim_start_matches("/api/runs/");
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Ok((400, "text/plain", b"Invalid id".to_vec()));
            }
            let update: UpdateRun = match serde_json::from_slice(body) {
                Ok(value) => value,
                Err(_) => return Ok((400, "text/plain", b"Invalid update".to_vec())),
            };
            let mut runs = load_runs(path)?;
            let Some(run) = runs.iter_mut().find(|r| r.id == id) else {
                return Ok((404, "text/plain", b"Run not found".to_vec()));
            };
            run.stage = update.stage;
            run.status = update.status;
            let result = serde_json::to_vec(run)?;
            save_runs(path, &runs)?;
            return Ok((200, "application/json", result));
        }
        Ok((404, "text/plain", b"Not found".to_vec()))
    })();
    result.unwrap_or_else(|e| {
        eprintln!("Run storage error: {e}");
        (500, "text/plain", b"Run storage error".to_vec())
    })
}

fn write_response(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(body)
}

fn api_post(route: &str, body: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(ADDRESS).map_err(|e| io::Error::new(e.kind(), format!("cannot connect to factory server at {ADDRESS}; start it with 'factory2 dashboard': {e}")))?;
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
    use std::thread;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "factory2-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn request(path: &Path, request: &[u8]) -> (u16, Vec<u8>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let path = path.to_owned();
        let server = thread::spawn(move || respond(listener.accept().unwrap().0, &path).unwrap());
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(request).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        server.join().unwrap();
        let separator = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let status = std::str::from_utf8(&response[..separator])
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (status, response[separator + 4..].to_vec())
    }

    fn post(path: &Path, route: &str, body: &str, extra: &str) -> (u16, Vec<u8>) {
        let req = format!("POST {route} HTTP/1.1\r\nHost: 127.0.0.1:4311\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{body}", body.len());
        request(path, req.as_bytes())
    }

    #[test]
    fn runs_persist_across_requests_and_validate_updates() {
        let dir = TestDir::new();
        let path = dir.0.join("runs.json");
        let (status, body) = post(
            &path,
            "/api/runs",
            r#"{"title":"<task>","project":"/work"}"#,
            "",
        );
        assert_eq!(status, 201);
        let run: Run = serde_json::from_slice(&body).unwrap();
        let (status, _) = post(
            &path,
            &format!("/api/runs/{}", run.id),
            r#"{"stage":"check","status":"running"}"#,
            "",
        );
        assert_eq!(status, 200);
        let (status, body) = request(
            &path,
            b"GET /api/runs HTTP/1.1\r\nHost: localhost:4311\r\n\r\n",
        );
        assert_eq!(status, 200);
        let saved: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(saved[0]["stage"], "check");
        assert_eq!(saved[0]["title"], "<task>");
        assert_eq!(
            post(
                &path,
                &format!("/api/runs/{}", run.id),
                r#"{"stage":"bad","status":"done"}"#,
                ""
            )
            .0,
            400
        );
        assert_eq!(
            post(
                &path,
                "/api/runs/123",
                r#"{"stage":"finish","status":"done"}"#,
                ""
            )
            .0,
            404
        );
        assert_eq!(
            post(
                &path,
                "/api/runs",
                r#"{"title":"x","project":"/"}"#,
                "Origin: https://example.com\r\n"
            )
            .0,
            403
        );
        assert_eq!(
            request(
                &path,
                b"GET /api/runs HTTP/1.1\r\nHost: example.com\r\n\r\n"
            )
            .0,
            400
        );
    }
}
