use rocket::{
    data::{Limits, ToByteUnit},
    figment::Figment,
    http::{ContentType, Status as HttpStatus},
    request::{FromRequest, Outcome, Request},
    response::content::RawHtml,
    serde::json::Json,
    Build, Rocket, State,
};
use serde::{Deserialize, Serialize};
mod install;
use std::{
    env, fs,
    io::{self, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ADDRESS: &str = "127.0.0.1:4311";
const DASHBOARD: &str = include_str!("dashboard.html");
const HELP: &str = "TDS Factory\n\nUsage:\n  factory2 install --agent <codex|claude|copilot>\n  factory2 dashboard\n  factory2 run start <title>\n  factory2 run update <id> <define|build|check|finish> <running|blocked|done> [options]\n  factory2 --help | --version";
const UPDATE_HELP: &str = "Run update options:\n  --activity <text> | --clear-activity\n  --current-ticket <http(s)-url> | --clear-current-ticket\n  --blocking-reason <text> | --clear-blocking-reason\nOmitted fields are preserved; clear flags remove them.";
const BODY_LIMIT: u64 = 64 * 1024;

#[derive(Clone, Serialize, Deserialize)]
struct Run {
    id: String,
    title: String,
    project: String,
    stage: Stage,
    status: RunStatus,
    #[serde(default)]
    activity: Option<String>,
    #[serde(default)]
    current_ticket: Option<String>,
    #[serde(default)]
    blocking_reason: Option<String>,
    #[serde(default)]
    updated_at: Option<u64>,
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
enum RunStatus {
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
#[serde(deny_unknown_fields)]
struct UpdateRun {
    stage: Stage,
    status: RunStatus,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    activity: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    current_ticket: FieldUpdate,
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    blocking_reason: FieldUpdate,
}

// Missing fields preserve stored values; null clears them; strings replace them.
#[derive(Default, Serialize)]
#[serde(untagged)]
enum FieldUpdate {
    #[default]
    Keep,
    Clear,
    Set(String),
}

impl<'de> Deserialize<'de> for FieldUpdate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<String>::deserialize(deserializer)? {
            Some(value) => Self::Set(value),
            None => Self::Clear,
        })
    }
}

impl FieldUpdate {
    fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    fn apply(self, target: &mut Option<String>) {
        match self {
            Self::Keep => {}
            Self::Clear => *target = None,
            Self::Set(value) => *target = Some(value),
        }
    }

    fn validate(&self, limit: usize) -> Result<(), &'static str> {
        if let Self::Set(value) = self {
            if value.trim().is_empty() || value.len() > limit {
                return Err("Run metadata must be nonblank and within its size limit");
            }
        }
        Ok(())
    }
}

impl UpdateRun {
    fn validate(&self) -> Result<(), &'static str> {
        self.activity.validate(2000)?;
        self.blocking_reason.validate(2000)?;
        self.current_ticket.validate(4096)?;
        if let FieldUpdate::Set(value) = &self.current_ticket {
            let valid = rocket::http::uri::Absolute::parse(value)
                .ok()
                .is_some_and(|uri| {
                    matches!(uri.scheme(), "http" | "https")
                        && uri
                            .authority()
                            .is_some_and(|authority| !authority.host().is_empty())
                });
            if !valid {
                return Err("Current ticket must be an absolute HTTP(S) URL");
            }
        }
        Ok(())
    }
}

fn parse_update(
    stage: &str,
    status: &str,
    options: &[&str],
) -> Result<UpdateRun, Box<dyn std::error::Error>> {
    let mut update = UpdateRun {
        stage: serde_json::from_value(stage.into())?,
        status: serde_json::from_value(status.into())?,
        activity: FieldUpdate::Keep,
        current_ticket: FieldUpdate::Keep,
        blocking_reason: FieldUpdate::Keep,
    };
    let mut args = options.iter();
    while let Some(flag) = args.next() {
        let field = match *flag {
            "--activity" | "--clear-activity" => &mut update.activity,
            "--current-ticket" | "--clear-current-ticket" => &mut update.current_ticket,
            "--blocking-reason" | "--clear-blocking-reason" => &mut update.blocking_reason,
            _ => return Err(format!("Unknown run update option: {flag}\n{UPDATE_HELP}").into()),
        };
        if !field.is_keep() {
            return Err(format!("Repeated or conflicting run update option: {flag}").into());
        }
        *field = if flag.starts_with("--clear-") {
            FieldUpdate::Clear
        } else {
            let value = args
                .next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("Missing value for {flag}\n{UPDATE_HELP}"))?;
            FieldUpdate::Set((*value).to_string())
        };
    }
    update.validate()?;
    Ok(update)
}

fn updated_now() -> io::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    u64::try_from(millis).map_err(io::Error::other)
}

struct RunStore {
    path: PathBuf,
    runs: Mutex<Vec<Run>>,
}

impl RunStore {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: path.to_owned(),
            runs: Mutex::new(load_runs(path)?),
        })
    }

    fn list(&self) -> io::Result<Vec<Run>> {
        Ok(self.runs.lock().map_err(lock_error)?.clone())
    }

    fn create(&self, new: NewRun) -> io::Result<Run> {
        let mut runs = self.runs.lock().map_err(lock_error)?;
        let mut id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        while runs.iter().any(|run| run.id == id.to_string()) {
            id += 1;
        }
        let run = Run {
            id: id.to_string(),
            title: new.title,
            project: new.project,
            stage: Stage::Define,
            status: RunStatus::Running,
            activity: None,
            current_ticket: None,
            blocking_reason: None,
            updated_at: Some(updated_now()?),
        };
        runs.push(run.clone());
        if let Err(error) = save_runs(&self.path, &runs) {
            runs.pop();
            return Err(error);
        }
        Ok(run)
    }

    fn update(&self, id: &str, update: UpdateRun) -> io::Result<Option<Run>> {
        let mut runs = self.runs.lock().map_err(lock_error)?;
        let Some(index) = runs.iter().position(|run| run.id == id) else {
            return Ok(None);
        };
        let updated_at = updated_now()?;
        let old = runs[index].clone();
        runs[index].stage = update.stage;
        runs[index].status = update.status;
        update.activity.apply(&mut runs[index].activity);
        update.current_ticket.apply(&mut runs[index].current_ticket);
        update
            .blocking_reason
            .apply(&mut runs[index].blocking_reason);
        runs[index].updated_at = Some(updated_at);
        if let Err(error) = save_runs(&self.path, &runs) {
            runs[index] = old;
            return Err(error);
        }
        Ok(Some(runs[index].clone()))
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> io::Error {
    io::Error::other("run storage lock was poisoned")
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
        [] | ["--help" | "-h"] => println!("{HELP}\n\n{UPDATE_HELP}"),
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
        ["run", "update", id, stage, status, options @ ..] => {
            if !id.bytes().all(|b| b.is_ascii_digit()) || id.is_empty() {
                return Err("Invalid run id".into());
            }
            let update = parse_update(stage, status, options)?;
            api_post(&format!("/api/runs/{id}"), &serde_json::to_vec(&update)?)?;
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

fn is_local_authority(authority: &str) -> bool {
    matches!(authority, "localhost:4311" | "127.0.0.1:4311")
}

struct Security {
    local: bool,
    post_allowed: bool,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Security {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let local = request
            .headers()
            .get_one("Host")
            .is_some_and(is_local_authority);
        Outcome::Success(Self {
            local,
            post_allowed: local
                && request.headers().get_one("Origin").is_none()
                && request.content_type() == Some(&ContentType::JSON),
        })
    }
}

#[rocket::get("/")]
fn dashboard(request: Security) -> Result<RawHtml<&'static str>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    Ok(RawHtml(DASHBOARD))
}

#[rocket::get("/api/runs")]
fn list_runs(request: Security, store: &State<RunStore>) -> Result<Json<Vec<Run>>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    store.list().map(Json).map_err(|error| {
        eprintln!("Run storage error: {error}");
        HttpStatus::InternalServerError
    })
}

#[rocket::post("/api/runs", data = "<new>")]
fn create_run(
    request: Security,
    new: Json<NewRun>,
    store: &State<RunStore>,
) -> Result<(HttpStatus, Json<Run>), HttpStatus> {
    if !request.post_allowed {
        return Err(HttpStatus::Forbidden);
    }
    if new.title.trim().is_empty()
        || new.project.trim().is_empty()
        || new.title.len() > 500
        || new.project.len() > 4096
    {
        return Err(HttpStatus::BadRequest);
    }
    store
        .create(new.into_inner())
        .map(|run| (HttpStatus::Created, Json(run)))
        .map_err(|error| {
            eprintln!("Run storage error: {error}");
            HttpStatus::InternalServerError
        })
}

#[rocket::post("/api/runs/<id>", data = "<update>")]
fn update_run(
    request: Security,
    id: &str,
    update: Json<UpdateRun>,
    store: &State<RunStore>,
) -> Result<Json<Run>, HttpStatus> {
    if !request.post_allowed {
        return Err(HttpStatus::Forbidden);
    }
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HttpStatus::BadRequest);
    }
    update.validate().map_err(|_| HttpStatus::BadRequest)?;
    store
        .update(id, update.into_inner())
        .map_err(|error| {
            eprintln!("Run storage error: {error}");
            HttpStatus::InternalServerError
        })?
        .map(Json)
        .ok_or(HttpStatus::NotFound)
}

#[rocket::catch(413)]
fn too_large() -> (HttpStatus, &'static str) {
    (HttpStatus::PayloadTooLarge, "Request too large")
}

#[rocket::catch(400)]
fn bad_request() -> (HttpStatus, &'static str) {
    (HttpStatus::BadRequest, "Bad request")
}

#[rocket::catch(422)]
fn invalid_entity() -> (HttpStatus, &'static str) {
    (HttpStatus::BadRequest, "Invalid JSON")
}

#[rocket::catch(404)]
fn not_found() -> (HttpStatus, &'static str) {
    (HttpStatus::NotFound, "Not found")
}

fn build_rocket(path: &Path, port: u16) -> io::Result<Rocket<Build>> {
    let config = Figment::from(rocket::Config::default())
        .merge(("address", "127.0.0.1"))
        .merge(("port", port))
        .merge(("limits", Limits::new().limit("json", BODY_LIMIT.bytes())));
    Ok(rocket::custom(config)
        .manage(RunStore::open(path)?)
        .mount(
            "/",
            rocket::routes![dashboard, list_runs, create_run, update_run],
        )
        .register(
            "/",
            rocket::catchers![too_large, bad_request, invalid_entity, not_found],
        )
        .attach(rocket::fairing::AdHoc::on_response(
            "No-store responses",
            |_, response| {
                Box::pin(async move {
                    response.set_header(rocket::http::Header::new("Cache-Control", "no-store"));
                })
            },
        )))
}

fn serve(path: &Path) -> io::Result<()> {
    let runtime = rocket::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    println!("Dashboard: http://{ADDRESS} (press Ctrl+C to stop)");
    runtime.block_on(async {
        build_rocket(path, 4311)
            .map_err(|error| {
                io::Error::new(error.kind(), format!("could not open run storage: {error}"))
            })?
            .launch()
            .await
            .map(|_| ())
            .map_err(io::Error::other)
    })
}

fn api_post(route: &str, body: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(ADDRESS).map_err(|e| {
        io::Error::new(e.kind(), format!("cannot connect to factory server at {ADDRESS}; start it with 'factory2 dashboard': {e}"))
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
    use rocket::{http::Header, local::asynchronous::Client};

    struct TestDir(tempfile::TempDir);
    impl TestDir {
        fn new() -> Self {
            Self(tempfile::tempdir().unwrap())
        }
    }

    async fn client(dir: &TestDir) -> Client {
        Client::tracked(build_rocket(&dir.0.path().join("runs.json"), 0).unwrap())
            .await
            .unwrap()
    }

    #[rocket::async_test]
    async fn dashboard_page_shows_stage_groups_and_last_updated() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let response = api
            .get("/")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::Ok);
        let page = response.into_string().await.unwrap();
        for group in ["Define", "Build", "Check", "Finish", "Done"] {
            assert!(
                page.contains(&format!("'{group}'")),
                "missing group {group}"
            );
        }
        assert!(page.contains("Last updated"));
    }

    #[rocket::async_test]
    async fn create_update_and_restart_preserve_cli_json_contract() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let response = api
            .post("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(r#"{"title":"<task>","project":"/work"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::Created);
        let created: Run = response.into_json().await.unwrap();
        assert_eq!(created.stage as u8, Stage::Define as u8);
        assert!(created.updated_at.is_some());
        let response = api
            .post(format!("/api/runs/{}", created.id))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(r#"{"stage":"check","status":"running"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::Ok);
        let restarted = client(&dir).await;
        let saved: serde_json::Value = restarted
            .get("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_json()
            .await
            .unwrap();
        assert_eq!(saved[0]["id"], created.id);
        assert_eq!(saved[0]["title"], "<task>");
        assert_eq!(saved[0]["stage"], "check");
        assert_eq!(
            serde_json::from_slice::<Vec<Run>>(&fs::read(dir.0.path().join("runs.json")).unwrap())
                .unwrap()
                .len(),
            1
        );
    }

    #[rocket::async_test]
    async fn rejects_bad_host_origin_content_type_and_invalid_json_without_mutation() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let host = api
            .get("/api/runs")
            .header(Header::new("Host", "attacker.example"))
            .dispatch()
            .await;
        assert_eq!(host.status(), HttpStatus::BadRequest);
        let wrong_port = api
            .get("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4312"))
            .dispatch()
            .await;
        assert_eq!(wrong_port.status(), HttpStatus::BadRequest);
        let origin = api
            .post("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .header(Header::new("Origin", "https://example.test"))
            .body(r#"{"title":"x","project":"/"}"#)
            .dispatch()
            .await;
        assert_eq!(origin.status(), HttpStatus::Forbidden);
        let content_type = api
            .post("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::Plain)
            .body(r#"{"title":"x","project":"/"}"#)
            .dispatch()
            .await;
        assert_eq!(content_type.status(), HttpStatus::Forbidden);
        let invalid = api
            .post("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body("{")
            .dispatch()
            .await;
        assert_eq!(invalid.status(), HttpStatus::BadRequest);
        let saved: Vec<Run> = api
            .get("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_json()
            .await
            .unwrap();
        assert!(saved.is_empty());
        assert_eq!(
            fs::read(dir.0.path().join("runs.json")).unwrap_or_default(),
            b""
        );
    }

    #[rocket::async_test]
    async fn enforces_body_limit_and_invalid_ids() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let big = format!(
            "{{\"title\":\"{}\",\"project\":\"/\"}}",
            "x".repeat(BODY_LIMIT as usize)
        );
        let response = api
            .post("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(big)
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::PayloadTooLarge);
        let response = api
            .post("/api/runs/nope")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(r#"{"stage":"check","status":"done"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::BadRequest);
        assert_eq!(
            api.post("/api/runs/123")
                .header(Header::new("Host", "127.0.0.1:4311"))
                .header(ContentType::JSON)
                .body(r#"{"stage":"check","status":"done"}"#)
                .dispatch()
                .await
                .status(),
            HttpStatus::NotFound
        );
    }

    #[test]
    fn concurrent_creates_keep_all_records_and_unique_ids() {
        let dir = TestDir::new();
        let store = std::sync::Arc::new(RunStore::open(&dir.0.path().join("runs.json")).unwrap());
        let threads: Vec<_> = (0..16)
            .map(|n| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .create(NewRun {
                            title: format!("run {n}"),
                            project: "/work".into(),
                        })
                        .unwrap()
                })
            })
            .collect();
        let created: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let ids: std::collections::HashSet<_> = created.iter().map(|run| run.id.clone()).collect();
        assert_eq!(ids.len(), 16);
        assert_eq!(
            load_runs(&dir.0.path().join("runs.json")).unwrap().len(),
            16
        );
    }

    #[test]
    fn concurrent_updates_to_different_runs_are_both_persisted() {
        let dir = TestDir::new();
        let store = std::sync::Arc::new(RunStore::open(&dir.0.path().join("runs.json")).unwrap());
        let first = store
            .create(NewRun {
                title: "first".into(),
                project: "/work".into(),
            })
            .unwrap();
        let second = store
            .create(NewRun {
                title: "second".into(),
                project: "/work".into(),
            })
            .unwrap();
        let updates = [(first.id, Stage::Build), (second.id, Stage::Check)];
        let threads: Vec<_> = updates
            .into_iter()
            .map(|(id, stage)| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .update(
                            &id,
                            UpdateRun {
                                stage,
                                status: RunStatus::Done,
                                activity: FieldUpdate::Keep,
                                current_ticket: FieldUpdate::Keep,
                                blocking_reason: FieldUpdate::Keep,
                            },
                        )
                        .unwrap()
                        .unwrap()
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let persisted = load_runs(&dir.0.path().join("runs.json")).unwrap();
        assert_eq!(persisted.len(), 2);
        assert!(persisted
            .iter()
            .all(|run| matches!(run.status, RunStatus::Done)));
        assert!(matches!(
            persisted
                .iter()
                .find(|run| run.title == "first")
                .unwrap()
                .stage,
            Stage::Build
        ));
        assert!(matches!(
            persisted
                .iter()
                .find(|run| run.title == "second")
                .unwrap()
                .stage,
            Stage::Check
        ));
    }
    async fn report<'a>(
        api: &'a Client,
        id: &str,
        body: &str,
    ) -> rocket::local::asynchronous::LocalResponse<'a> {
        api.post(format!("/api/runs/{id}"))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(body)
            .dispatch()
            .await
    }

    #[rocket::async_test]
    async fn old_records_report_preserve_clear_and_persist_metadata() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        fs::write(
            &path,
            r#"[{"id":"42","title":"old","project":"/work","stage":"define","status":"running"}]"#,
        )
        .unwrap();
        let api = client(&dir).await;
        let original = api
            .rocket()
            .state::<RunStore>()
            .unwrap()
            .list()
            .unwrap()
            .remove(0);
        assert!(original.activity.is_none());
        assert!(original.current_ticket.is_none());
        assert!(original.blocking_reason.is_none());
        assert!(
            original.updated_at.is_none(),
            "legacy records have no invented time"
        );

        let before = updated_now().unwrap();
        let response = report(&api, "42", r#"{"stage":"build","status":"blocked","activity":"Implementing #42","current_ticket":"https://github.com/example/repo/issues/42","blocking_reason":"Need credentials"}"#).await;
        assert_eq!(response.status(), HttpStatus::Ok);
        let run: Run = response.into_json().await.unwrap();
        assert!((before..=updated_now().unwrap()).contains(&run.updated_at.unwrap()));
        assert_eq!(run.activity.as_deref(), Some("Implementing #42"));
        assert_eq!(run.blocking_reason.as_deref(), Some("Need credentials"));
        assert_eq!(
            run.current_ticket.as_deref(),
            Some("https://github.com/example/repo/issues/42")
        );

        let restarted = client(&dir).await;
        let response = report(
            &restarted,
            "42",
            r#"{"stage":"build","status":"running","blocking_reason":null}"#,
        )
        .await;
        assert_eq!(response.status(), HttpStatus::Ok);
        let resumed: Run = response.into_json().await.unwrap();
        assert_eq!(
            resumed.activity, run.activity,
            "omitted fields stay unchanged"
        );
        assert_eq!(resumed.current_ticket, run.current_ticket);
        assert!(resumed.blocking_reason.is_none());
        assert!(resumed.updated_at.unwrap() >= run.updated_at.unwrap());
        let response = report(
            &restarted,
            "42",
            r#"{"stage":"finish","status":"done","activity":null,"current_ticket":null}"#,
        )
        .await;
        assert_eq!(response.status(), HttpStatus::Ok);
        let persisted = load_runs(&path).unwrap().remove(0);
        assert!(persisted.activity.is_none());
        assert!(persisted.current_ticket.is_none());
        assert!(persisted.blocking_reason.is_none());
        assert!(persisted.updated_at.is_some());
        assert!(matches!(persisted.status, RunStatus::Done));
    }

    #[rocket::async_test]
    async fn invalid_metadata_and_caller_timestamps_do_not_mutate_runs() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let store = api.rocket().state::<RunStore>().unwrap();
        let run = store
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
            })
            .unwrap();
        let path = dir.0.path().join("runs.json");
        let original = fs::read(&path).unwrap();
        for field in [
            serde_json::json!({"activity": " "}),
            serde_json::json!({"activity": "x".repeat(2001)}),
            serde_json::json!({"blocking_reason": "x".repeat(2001)}),
            serde_json::json!({"current_ticket": "https://example.com/".to_string() + &"x".repeat(4096)}),
            serde_json::json!({"current_ticket": "javascript:alert(1)"}),
            serde_json::json!({"current_ticket": "/relative"}),
            serde_json::json!({"current_ticket": "https:///missing-host"}),
            serde_json::json!({"activity": 42}),
            serde_json::json!({"updated_at": 1}),
        ] {
            let mut body = serde_json::json!({"stage":"build","status":"blocked"});
            body.as_object_mut()
                .unwrap()
                .extend(field.as_object().unwrap().clone());
            let response = report(&api, &run.id, &body.to_string()).await;
            assert_eq!(response.status(), HttpStatus::BadRequest, "input: {body}");
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(
                serde_json::to_vec(&store.list().unwrap()).unwrap(),
                original
            );
        }
    }

    #[test]
    fn cli_update_options_preserve_set_clear_and_reject_conflicts() {
        let update = parse_update("build", "running", &[]).unwrap();
        assert_eq!(
            serde_json::to_value(update).unwrap(),
            serde_json::json!({"stage":"build","status":"running"})
        );
        let update = parse_update(
            "build",
            "blocked",
            &[
                "--activity",
                "Implementing ticket",
                "--current-ticket",
                "https://example.com/42",
                "--blocking-reason",
                "Need input",
            ],
        )
        .unwrap();
        let value = serde_json::to_value(update).unwrap();
        assert_eq!(value["activity"], "Implementing ticket");
        assert_eq!(value["current_ticket"], "https://example.com/42");
        assert_eq!(value["blocking_reason"], "Need input");
        let update = parse_update(
            "finish",
            "done",
            &[
                "--clear-activity",
                "--clear-current-ticket",
                "--clear-blocking-reason",
            ],
        )
        .unwrap();
        let value = serde_json::to_value(update).unwrap();
        for field in ["activity", "current_ticket", "blocking_reason"] {
            assert_eq!(value.get(field), Some(&serde_json::Value::Null));
        }
        for options in [
            vec!["--activity"],
            vec!["--activity", "--clear-activity"],
            vec!["--clear-activity", "--activity", "x"],
            vec!["--clear-current-ticket", "--clear-current-ticket"],
            vec!["--unknown"],
            vec!["--current-ticket", "file:///tmp/ticket"],
            vec!["--blocking-reason", ""],
        ] {
            assert!(
                parse_update("build", "running", &options).is_err(),
                "options: {options:?}"
            );
        }
        assert!(parse_update("unknown", "running", &[]).is_err());
        assert!(parse_update("build", "unknown", &[]).is_err());
    }

    #[test]
    fn failed_save_rolls_back_metadata_and_timestamp() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = store
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
            })
            .unwrap();
        let original = fs::read(&path).unwrap();
        fs::create_dir(path.with_extension("json.tmp")).unwrap();
        let update = parse_update(
            "build",
            "blocked",
            &["--activity", "Working", "--blocking-reason", "Need input"],
        )
        .unwrap();
        assert!(store.update(&run.id, update).is_err());
        assert_eq!(
            serde_json::to_vec(&store.list().unwrap()).unwrap(),
            original
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
