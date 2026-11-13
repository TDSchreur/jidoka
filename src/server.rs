use crate::{
    current_build,
    document::{self, DocumentError},
    pr_state,
    pr_wait::{self, RealGh},
    run::{NewRun, Run, RunStore, UpdateRun},
    usage::{UsageSettings, collect_and_store},
};
use rocket::{
    Build, Rocket, State,
    data::{Limits, ToByteUnit},
    figment::Figment,
    http::{ContentType, Status as HttpStatus},
    request::{FromRequest, Outcome, Request},
    response::content::RawHtml,
    serde::json::Json,
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// The host the dashboard server binds to and the CLI talks to: loopback only, so nothing outside this machine
/// can reach it.
pub(crate) const LOOPBACK_HOST: &str = "127.0.0.1";
/// The port the dashboard tries first; when it is taken, the dashboard takes a free port instead.
pub(crate) const DEFAULT_PORT: u16 = 4311;
const DASHBOARD: &str = include_str!("../dashboard/index.html");
/// The files the dashboard page may load from `/assets/`; any other path is a 404.
const ASSETS: &[(&str, ContentType, &[u8])] = &[
    (
        "app.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/app.js"),
    ),
    (
        "favicon.svg",
        ContentType::SVG,
        include_bytes!("../dashboard/favicon.svg"),
    ),
    (
        "format.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/format.js"),
    ),
    (
        "help.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/help.js"),
    ),
    (
        "history.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/history.js"),
    ),
    (
        "style.css",
        ContentType::CSS,
        include_bytes!("../dashboard/style.css"),
    ),
    (
        "vendor/preact.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/vendor/preact.js"),
    ),
    (
        "vendor/hooks.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/vendor/hooks.js"),
    ),
    (
        "vendor/htm.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/vendor/htm.js"),
    ),
    (
        "vendor/barlow-400.woff2",
        ContentType::WOFF2,
        include_bytes!("../dashboard/vendor/barlow-400.woff2"),
    ),
    (
        "vendor/barlow-600.woff2",
        ContentType::WOFF2,
        include_bytes!("../dashboard/vendor/barlow-600.woff2"),
    ),
];
/// Placeholder in the dashboard page that is replaced with the build label.
const BUILD_PLACEHOLDER: &str = "__BUILD__";
/// Placeholder in the dashboard page that is replaced with this server's session token.
const TOKEN_PLACEHOLDER: &str = "__TOKEN__";
/// Placeholder in the dashboard page that is replaced with this server's process id, so the page can tell when a
/// new server is running and reload itself.
const PID_PLACEHOLDER: &str = "__PID__";
/// The header the dashboard sends its session token in.
const TOKEN_HEADER: &str = "X-Session-Token";
/// The header the `jidoka` CLI sends its CLI token in.
pub(crate) const CLI_TOKEN_HEADER: &str = "X-Jidoka-Token";
const BODY_LIMIT: u64 = 64 * 1024;

fn is_local_authority(authority: &str, port: u16) -> bool {
    authority.rsplit_once(':').is_some_and(|(host, given)| {
        matches!(host, "localhost" | LOOPBACK_HOST) && given == port.to_string()
    })
}

/// A random value created when the server starts. Only the dashboard page this server
/// serves to a local host carries it, so a request that has it came from that page.
struct SessionToken(String);

impl SessionToken {
    fn new() -> Result<Self, String> {
        crate::token::random().map(Self)
    }
}

/// A second random value, per server process, that the `jidoka` CLI needs to POST. It is
/// written to a file only the user can read and is never part of any response.
struct CliToken(String);

impl CliToken {
    /// Compares without stopping at the first differing byte.
    fn matches(&self, given: &str) -> bool {
        self.0.len() == given.len()
            && self
                .0
                .bytes()
                .zip(given.bytes())
                .fold(0, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

/// Where the CLI token lives: next to the runs file.
pub(crate) fn token_path(runs: &Path) -> PathBuf {
    runs.with_file_name("session-token")
}

/// The data folder: the folder that holds `runs.json`, and with it the CLI token, `dashboard.json` and plans.
pub(crate) fn data_dir(runs: &Path) -> &Path {
    runs.parent().unwrap_or(Path::new("."))
}

/// Writes `contents` to `path` so no reader sees a partial file; on Unix only the user can read it and the
/// directory is made private. Used for the CLI token and the discovery file.
pub(crate) fn write_private_file(path: &Path, contents: &str) -> io::Result<()> {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let first = COUNTER.fetch_add(TOKEN_TEMP_ATTEMPTS, std::sync::atomic::Ordering::Relaxed);
    write_private_file_from(path, contents, first)
}

/// How many temporary names a token write tries before it gives up.
const TOKEN_TEMP_ATTEMPTS: u32 = 8;

/// The temporary name for the `counter`-th attempt. It needs no secrecy: `create_new` refuses to reuse or
/// follow anything that exists.
fn token_temp_path(path: &Path, counter: u32) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    dir.join(format!(".{name}.{}.{counter}.tmp", std::process::id()))
}

/// `write_private_file`, trying the temporary names from `first_counter` on. A name left by a killed dashboard
/// (same pid, same counter) is skipped; after `TOKEN_TEMP_ATTEMPTS` taken names the error is returned.
fn write_private_file_from(path: &Path, contents: &str, first_counter: u32) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    let mut attempt = 0;
    loop {
        let temp = token_temp_path(path, first_counter.wrapping_add(attempt));
        match write_new_file(&temp, contents) {
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && attempt + 1 < TOKEN_TEMP_ATTEMPTS =>
            {
                attempt += 1;
            }
            Err(error) => return Err(error),
            Ok(()) => {
                let renamed = std::fs::rename(&temp, path);
                if renamed.is_err() {
                    // Best-effort cleanup; the rename error is the one returned.
                    let _ = std::fs::remove_file(&temp);
                }
                return renamed;
            }
        }
    }
}

/// Makes the data folder of `runs` private (0700) before anything is written into it: created that way
/// when missing, chmod-ed when it is a real folder. A symlinked folder is left as it is. Fails when the
/// folder cannot be made private, so the dashboard does not start with a readable token folder. Returns a
/// warning when the folder is a symlink whose target is group- or world-accessible.
#[cfg(unix)]
fn secure_data_dir(runs: &Path) -> io::Result<Option<String>> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let dir = match runs.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => return Ok(None),
    };
    let failed = |error: io::Error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot make the data folder {} private (mode 0700): {error}",
                dir.display()
            ),
        )
    };
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let open = std::fs::metadata(dir).map_err(failed)?.permissions().mode() & 0o077 != 0;
            Ok(open.then(|| {
                format!(
                    "warning: the data folder {} is a symlink to a folder that other users can access; the CLI token is stored there. Run: chmod 700 {}",
                    dir.display(),
                    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).display()
                )
            }))
        }
        Ok(_) => std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map(|()| None)
            .map_err(failed),
        Err(error) if error.kind() == io::ErrorKind::NotFound => std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map(|()| None)
            .map_err(failed),
        Err(error) => Err(failed(error)),
    }
}

/// Creates `path`, which must not exist (and is never a followed symlink), holding `content`; mode 0600 on Unix.
fn write_new_file(path: &Path, content: &str) -> io::Result<()> {
    use std::io::Write;
    write_new_file_with(path, content, |file, bytes| file.write_all(bytes))
}

/// `write_new_file` with the write step injected. When the write fails after the file was created, the
/// file is removed again; an open error never removes anything (an existing file is not ours).
fn write_new_file_with(
    path: &Path,
    content: &str,
    write: impl FnOnce(&mut std::fs::File, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let written = write(&mut file, content.as_bytes());
    if written.is_err() {
        drop(file);
        // Best-effort cleanup; the write error is the one returned.
        let _ = std::fs::remove_file(path);
    }
    written
}

/// The browser sends an `Origin` on a same-origin DELETE; it must be the dashboard itself.
fn is_local_origin(origin: &str, port: u16) -> bool {
    origin
        .strip_prefix("http://")
        .is_some_and(|authority| is_local_authority(authority, port))
}

struct Security {
    local: bool,
    /// Host, Origin and content type are acceptable for a POST.
    post_shape_ok: bool,
    /// The request carries the CLI token.
    cli_token_ok: bool,
    delete_allowed: bool,
}

impl Security {
    /// Why a POST is refused: 403 when Host, Origin or content type are not acceptable, 401 when only the
    /// CLI token is missing or wrong.
    fn post_refusal(&self) -> Option<HttpStatus> {
        if !self.post_shape_ok {
            Some(HttpStatus::Forbidden)
        } else if !self.cli_token_ok {
            Some(HttpStatus::Unauthorized)
        } else {
            None
        }
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Security {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let port = request.rocket().config().port;
        let local = request
            .headers()
            .get_one("Host")
            .is_some_and(|authority| is_local_authority(authority, port));
        let post_shape_ok = local
            && request.headers().get_one("Origin").is_none()
            && request.content_type() == Some(&ContentType::JSON);
        let cli_token_ok = request
            .rocket()
            .state::<CliToken>()
            .zip(request.headers().get_one(CLI_TOKEN_HEADER))
            .is_some_and(|(token, given)| token.matches(given));
        Outcome::Success(Self {
            local,
            post_shape_ok,
            cli_token_ok,
            delete_allowed: local
                && request
                    .headers()
                    .get_one("Origin")
                    .is_none_or(|origin| is_local_origin(origin, port))
                && request
                    .rocket()
                    .state::<SessionToken>()
                    .zip(request.headers().get_one(TOKEN_HEADER))
                    .is_some_and(|(token, given)| token.0 == given),
        })
    }
}

/// A POST that passed the Host, Origin, content type and CLI token checks. As a request guard it runs
/// before the body is read, so a caller without the token never reaches the JSON parser.
struct PostAccess;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for PostAccess {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match Security::from_request(request).await {
            Outcome::Success(security) => match security.post_refusal() {
                None => Outcome::Success(Self),
                Some(status) => Outcome::Error((status, ())),
            },
            Outcome::Error(error) => Outcome::Error(error),
            Outcome::Forward(status) => Outcome::Forward(status),
        }
    }
}

#[rocket::get("/")]
fn dashboard(
    request: Security,
    token: &State<SessionToken>,
) -> Result<RawHtml<String>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    Ok(RawHtml(
        DASHBOARD
            .replace(BUILD_PLACEHOLDER, &current_build())
            .replace(TOKEN_PLACEHOLDER, &token.0)
            .replace(PID_PLACEHOLDER, &std::process::id().to_string()),
    ))
}

#[rocket::get("/assets/<path..>")]
fn asset(request: Security, path: PathBuf) -> Result<(ContentType, &'static [u8]), HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    let name = path
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    ASSETS
        .iter()
        .find(|(known, _, _)| *known == name)
        .map(|(_, content_type, bytes)| (content_type.clone(), *bytes))
        .ok_or(HttpStatus::NotFound)
}

/// What a client needs to recognise a running Jidoka dashboard and compare its build with its own.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub(crate) struct DashboardIdentity {
    pub(crate) pid: u32,
    pub(crate) version: String,
}

#[rocket::get("/api/info")]
fn info(request: Security) -> Result<Json<DashboardIdentity>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    Ok(Json(DashboardIdentity {
        pid: std::process::id(),
        version: current_build(),
    }))
}

#[rocket::get("/api/runs")]
fn list_runs(
    request: Security,
    store: &State<RunStore>,
) -> Result<Json<Vec<serde_json::Value>>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    store
        .list()
        .map_err(|error| {
            eprintln!("Run storage error: {error}");
            HttpStatus::InternalServerError
        })?
        .iter()
        .map(run_with_cost)
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
        .map_err(|error| {
            eprintln!("Run cost serialization error: {error}");
            HttpStatus::InternalServerError
        })
}

/// A run as the API shows it: the stored record plus the estimated cost of its usage, from the price
/// table as it is now. The cost is never stored.
fn run_with_cost(run: &Run) -> serde_json::Result<serde_json::Value> {
    let mut value = serde_json::to_value(run)?;
    if let Some(usage) = &run.usage {
        value["usage"] = crate::cost::usage_json(usage, crate::cost::PRICES)?;
    }
    Ok(value)
}

/// Enrich a single run with gate and check status. Diff and HEAD inspection is kept off list polling.
fn run_detail(run: &Run) -> serde_json::Result<serde_json::Value> {
    let mut value = run_with_cost(run)?;
    value["stage_gates"] = serde_json::to_value(crate::run_next::stage_gates(run))?;
    if let Some(snapshot) = run.check_history.last() {
        value["recorded_skipped_checks"] = serde_json::to_value(&snapshot.skipped)?;
        value["recorded_skipped_rework"] = serde_json::to_value(snapshot.rework)?;
    } else if let Some(result) = run.gates.iter().rev().find(|result| result.check.is_some()) {
        value["recorded_skipped_checks"] = serde_json::to_value(&result.skipped_checks)?;
        value["recorded_skipped_rework"] = serde_json::to_value(result.rework)?;
    }
    let paths = match run.mode {
        None => Err("Record the workflow mode to select checks".to_string()),
        Some(crate::run::Mode::Fast) => Ok(Vec::new()),
        Some(_) => crate::run_next::changed_paths(run, &crate::run_check::ReadOnlyGit),
    };
    match paths {
        Ok(paths) => {
            value["checks"] = serde_json::to_value(crate::run_next::checks_for(
                run.mode.expect("a mode is required for check selection"),
                &paths,
                run.model_routing.as_ref(),
            ))?;
        }
        Err(reason) => value["checks_unavailable"] = serde_json::Value::String(reason),
    }
    Ok(value)
}

#[rocket::post("/api/runs", data = "<new>")]
fn create_run(
    _access: PostAccess,
    new: Json<NewRun>,
    store: &State<RunStore>,
) -> Result<(HttpStatus, Json<Run>), HttpStatus> {
    store
        .create(new.into_inner())
        .map(|run| (HttpStatus::Created, Json(run)))
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                HttpStatus::BadRequest
            } else {
                eprintln!("Run storage error: {error}");
                HttpStatus::InternalServerError
            }
        })
}

#[rocket::post("/api/runs/<id>", data = "<update>")]
fn update_run(
    _access: PostAccess,
    id: &str,
    update: Json<UpdateRun>,
    store: &State<RunStore>,
    collector: &State<Collector>,
) -> Result<Json<Run>, (HttpStatus, String)> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err((HttpStatus::BadRequest, "Invalid run id".into()));
    }
    let updated = store
        .update(id, update.into_inner())
        .map_err(|error| match error.kind() {
            // An inconsistent update (an unknown ticket, two tickets in progress) is the caller's to fix.
            io::ErrorKind::InvalidInput => (HttpStatus::BadRequest, error.to_string()),
            _ => {
                eprintln!("Run storage error: {error}");
                (
                    HttpStatus::InternalServerError,
                    "Internal server error".into(),
                )
            }
        })?
        .ok_or((HttpStatus::NotFound, "Not found".into()))?;
    collector.after_update(store, &updated.id);
    Ok(Json(updated))
}

#[rocket::post("/api/runs/<id>/gate", data = "<record>")]
fn record_gate(
    _access: PostAccess,
    id: &str,
    record: Json<crate::run_gate::Record>,
    store: &State<RunStore>,
) -> Result<&'static str, (HttpStatus, String)> {
    store
        .record_gate(id, record.into_inner(), &crate::pr_sync::RealGit)
        .map_err(|error| (HttpStatus::BadRequest, error))?;
    Ok("Gate recorded")
}

#[rocket::post("/api/runs/<id>/advance", data = "<request>")]
fn advance_run(
    _access: PostAccess,
    id: &str,
    request: Json<crate::run_gate::AdvanceRequest>,
    store: &State<RunStore>,
    collector: &State<Collector>,
) -> Result<&'static str, (HttpStatus, String)> {
    let request = request.into_inner();
    let forced = request
        .forced_reason()
        .map_err(|error| (HttpStatus::BadRequest, error))?;
    store
        .change_gate(id, |run| match forced {
            Some(reason) => crate::run_gate::advance_forced(run, reason),
            None => {
                crate::run_gate::advance(run, &crate::pr_sync::RealGit, &crate::pr_wait::RealGh)
            }
        })
        .map_err(|error| (HttpStatus::BadRequest, error))?;
    collector.after_update(store, id);
    Ok("Run advanced")
}

#[rocket::get("/api/runs/<id>")]
fn get_run(
    request: Security,
    id: &str,
    store: &State<RunStore>,
) -> Result<Json<serde_json::Value>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HttpStatus::BadRequest);
    }
    store
        .get(id)
        .map_err(|error| {
            eprintln!("Run storage error: {error}");
            HttpStatus::InternalServerError
        })?
        .ok_or(HttpStatus::NotFound)
        .and_then(|run| {
            run_detail(&run).map(Json).map_err(|error| {
                eprintln!("Run cost serialization error: {error}");
                HttpStatus::InternalServerError
            })
        })
}

/// The HTTP answer for a document that is not served: `404` with
/// `{"available": false, "path": <reference>}` when the recorded file is not there (yet), any other
/// code with `{"error": <reason>}` when the request is refused.
type DocumentFailure = (HttpStatus, Json<serde_json::Value>);

fn refused(status: HttpStatus, reason: &str) -> DocumentFailure {
    (status, Json(serde_json::json!({ "error": reason })))
}

fn document_failure(error: DocumentError) -> DocumentFailure {
    match error {
        DocumentError::NotRecorded => refused(
            HttpStatus::Forbidden,
            "Not a reference recorded on this run",
        ),
        DocumentError::WebReference => {
            refused(HttpStatus::BadRequest, "Only local documents are served")
        }
        DocumentError::NotMarkdown => refused(
            HttpStatus::BadRequest,
            "Only Markdown (.md) documents are served",
        ),
        DocumentError::OutsideRoots => refused(
            HttpStatus::Forbidden,
            "The document is outside the run's worktree and repository",
        ),
        DocumentError::TooLarge => refused(
            HttpStatus::PayloadTooLarge,
            "The document is larger than 1 MB",
        ),
        DocumentError::NotText => {
            refused(HttpStatus::UnprocessableEntity, "The document is not text")
        }
        DocumentError::NotAvailable { path } => (
            HttpStatus::NotFound,
            Json(serde_json::json!({ "available": false, "path": path })),
        ),
    }
}

/// A recorded local spec or ticket of a run as rendered HTML: `{"name": <file name>, "html": <HTML>}`.
/// See `document::read` for the refusals and the not-available answer.
#[rocket::get("/api/runs/<id>/document?<reference>")]
async fn get_document(
    request: Security,
    id: &str,
    reference: &str,
    store: &State<RunStore>,
    runs_file: &State<TokenFile>,
) -> Result<Json<serde_json::Value>, DocumentFailure> {
    if !request.local {
        return Err(refused(HttpStatus::BadRequest, "Bad request"));
    }
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(refused(HttpStatus::BadRequest, "Invalid run id"));
    }
    let run = store
        .get(id)
        .map_err(|error| {
            eprintln!("Run storage error: {error}");
            refused(HttpStatus::InternalServerError, "Internal server error")
        })?
        .ok_or_else(|| refused(HttpStatus::NotFound, "Not found"))?;
    let reference = reference.to_owned();
    let plan = crate::plans::plan_path(&runs_file.0, &run.id);
    let read = rocket::tokio::task::spawn_blocking(move || document::read(&run, &reference, &plan))
        .await
        .map_err(|error| {
            eprintln!("Document read stopped: {error}");
            refused(HttpStatus::InternalServerError, "Internal server error")
        })?;
    let document = read.map_err(document_failure)?;
    Ok(Json(
        serde_json::json!({ "name": document.name, "html": document.html }),
    ))
}

/// Deletes the Run record only; nothing the Run created (worktree, branch, pull request) is touched.
#[rocket::delete("/api/runs/<id>")]
fn delete_run(
    request: Security,
    id: &str,
    store: &State<RunStore>,
) -> Result<HttpStatus, HttpStatus> {
    if !request.delete_allowed {
        return Err(HttpStatus::Forbidden);
    }
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HttpStatus::BadRequest);
    }
    match store.delete(id) {
        Ok(true) => Ok(HttpStatus::NoContent),
        Ok(false) => Err(HttpStatus::NotFound),
        Err(error) => {
            eprintln!("Run storage error: {error}");
            Err(HttpStatus::InternalServerError)
        }
    }
}

/// How long a sync may wait for the host before the dashboard gives up on it.
const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Reads the state of the Run's pull request from its host now, and brings the Run in line with it: a merged
/// pull request finishes the Run, a closed one blocks it. Needs the session token, like delete.
#[rocket::post("/api/runs/<id>/sync")]
async fn sync_run(
    request: Security,
    id: &str,
    store: &State<RunStore>,
    tool: &State<PrTool>,
    collector: &State<Collector>,
) -> Result<Json<pr_state::PrStatus>, HttpStatus> {
    if !request.delete_allowed {
        return Err(HttpStatus::Forbidden);
    }
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HttpStatus::BadRequest);
    }
    let (read_store, read_tool, read_id) = (store.inner().clone(), tool.0.clone(), id.to_owned());
    let read = rocket::tokio::task::spawn_blocking(move || {
        let now = crate::run::updated_now()?;
        pr_state::sync_run(&read_store, read_tool.as_ref(), &read_id, now)
    });
    let status = match rocket::tokio::time::timeout(SYNC_TIMEOUT, read).await {
        Ok(Ok(Ok(Some(status)))) => status,
        Ok(Ok(Ok(None))) => return Err(HttpStatus::NotFound),
        Ok(Ok(Err(error))) => {
            eprintln!("Run storage error: {error}");
            return Err(HttpStatus::InternalServerError);
        }
        Ok(Err(error)) => {
            eprintln!("Pull request sync stopped: {error}");
            return Err(HttpStatus::InternalServerError);
        }
        Err(_) => return Err(HttpStatus::GatewayTimeout),
    };
    if matches!(
        status.state,
        pr_state::PrState::Merged | pr_state::PrState::Closed
    ) {
        collector.after_update(store, id);
    }
    Ok(Json(status))
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

/// Collects token usage in the background after an update that moves a run on. Without settings
/// (as in most tests) nothing is collected.
pub(crate) struct Collector {
    settings: Option<UsageSettings>,
    pending: Mutex<Vec<rocket::tokio::task::JoinHandle<()>>>,
    /// The latest collection chain per run: a newer one replaces an older one's later passes, so a flurry
    /// of updates does not pile up scans.
    chains: std::sync::Arc<Mutex<std::collections::HashMap<String, u64>>>,
    /// How many passes ran, for tests.
    #[cfg(test)]
    passes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Collector {
    pub(crate) fn new(settings: Option<UsageSettings>) -> Self {
        Self {
            settings,
            pending: Mutex::new(Vec::new()),
            chains: std::sync::Arc::default(),
            #[cfg(test)]
            passes: std::sync::Arc::default(),
        }
    }

    /// Starts background collection after a successful update requires it.
    fn after_update(&self, store: &RunStore, id: &str) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let (store, id) = (store.clone(), id.to_owned());
        let chains = self.chains.clone();
        let generation = chains.lock().map_or(0, |mut chains| {
            let latest = chains.entry(id.clone()).or_insert(0);
            *latest += 1;
            *latest
        });
        #[cfg(test)]
        let passes = self.passes.clone();
        let handle = rocket::tokio::spawn(async move {
            // One pass now, and one after each delay: the agents' logs lag behind the work.
            let delays = std::iter::once(std::time::Duration::ZERO).chain(settings.settle.clone());
            for delay in delays {
                if !delay.is_zero() {
                    rocket::tokio::time::sleep(delay).await;
                }
                // A newer update started its own chain, which does these passes again.
                if chains
                    .lock()
                    .is_ok_and(|chains| chains.get(&id) != Some(&generation))
                {
                    return;
                }
                let (store, id, settings) = (store.clone(), id.clone(), settings.clone());
                #[cfg(test)]
                passes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let joined = rocket::tokio::task::spawn_blocking({
                    let id = id.clone();
                    move || {
                        if let Err(error) = collect_and_store(&store, &id, &settings) {
                            eprintln!("Usage was not collected for run {id}: {error}");
                        }
                    }
                })
                .await;
                if let Err(error) = joined {
                    eprintln!(
                        "Usage was not collected for run {id}: the collection stopped: {error}"
                    );
                }
            }
            if let Ok(mut chains) = chains.lock()
                && chains.get(&id) == Some(&generation)
            {
                chains.remove(&id);
            }
        });
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|task| !task.is_finished());
            pending.push(handle);
        }
    }

    /// Waits for the collections started so far, for tests.
    #[cfg(test)]
    pub(crate) async fn wait_idle(&self) {
        let handles: Vec<_> = self
            .pending
            .lock()
            .map(|mut p| p.drain(..).collect())
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.await;
        }
    }
}

/// The tool that reads pull requests from their hosts, shared by the background pass and the sync endpoint.
struct PrTool(std::sync::Arc<dyn pr_wait::Gh + Send + Sync>);

#[cfg(test)]
pub(crate) fn build_rocket(path: &Path, port: u16) -> io::Result<Rocket<Build>> {
    build_rocket_with_tokens(
        path,
        port,
        Collector::new(None),
        new_tokens()?,
        std::sync::Arc::new(RealGh),
    )
}

#[cfg(test)]
fn build_rocket_with_tool(
    path: &Path,
    port: u16,
    tool: std::sync::Arc<dyn pr_wait::Gh + Send + Sync>,
) -> io::Result<Rocket<Build>> {
    build_rocket_with_tokens(path, port, Collector::new(None), new_tokens()?, tool)
}

/// The runs file the CLI token is kept next to.
struct TokenFile(PathBuf);

/// Why the server shut itself down at liftoff, if it did; `serve` reports it.
enum LiftoffStop {
    /// Another Jidoka dashboard is already running.
    AlreadyRunning,
    /// The CLI token or the discovery file could not be written.
    Failed(String),
}

/// Set at liftoff when the server stops itself.
struct LiftoffStopSlot(std::sync::Mutex<Option<LiftoffStop>>);

#[cfg(test)]
pub(crate) fn build_rocket_with(
    path: &Path,
    port: u16,
    collector: Collector,
) -> io::Result<Rocket<Build>> {
    build_rocket_with_tokens(
        path,
        port,
        collector,
        new_tokens()?,
        std::sync::Arc::new(RealGh),
    )
}

/// The session token and the CLI token, or why the operating system could not provide them.
fn new_tokens() -> io::Result<(SessionToken, String)> {
    let session_token = SessionToken::new().map_err(io::Error::other)?;
    let cli_token = crate::token::random().map_err(io::Error::other)?;
    Ok((session_token, cli_token))
}

fn build_rocket_with_tokens(
    path: &Path,
    port: u16,
    collector: Collector,
    (session_token, cli_token): (SessionToken, String),
    tool: std::sync::Arc<dyn pr_wait::Gh + Send + Sync>,
) -> io::Result<Rocket<Build>> {
    let config = Figment::from(rocket::Config::default())
        .merge(("address", LOOPBACK_HOST))
        .merge(("port", port))
        .merge(("limits", Limits::new().limit("json", BODY_LIMIT.bytes())));
    #[cfg(unix)]
    if let Some(warning) = secure_data_dir(path)? {
        eprintln!("{warning}");
    }
    Ok(rocket::custom(config)
        .manage(RunStore::open(path)?)
        .manage(collector)
        .manage(PrTool(tool))
        .manage(session_token)
        .manage(CliToken(cli_token))
        .manage(TokenFile(path.to_path_buf()))
        .manage(LiftoffStopSlot(std::sync::Mutex::new(None)))
        .attach(rocket::fairing::AdHoc::on_liftoff(
            "Announce dashboard",
            |rocket| {
                Box::pin(async move {
                    // Only once the port is bound, so a dashboard that fails to bind leaves the files alone.
                    let (Some(token), Some(file), Some(slot)) = (
                        rocket.state::<CliToken>(),
                        rocket.state::<TokenFile>(),
                        rocket.state::<LiftoffStopSlot>(),
                    ) else {
                        return;
                    };
                    if let Some(stop) = announce(&file.0, &token.0, rocket.config().port) {
                        if let LiftoffStop::Failed(message) = &stop {
                            eprintln!("{message}");
                        }
                        if let Ok(mut slot) = slot.0.lock() {
                            *slot = Some(stop);
                        }
                        rocket.shutdown().notify();
                    }
                })
            },
        ))
        .mount(
            "/",
            rocket::routes![
                dashboard,
                asset,
                info,
                list_runs,
                get_run,
                get_document,
                create_run,
                update_run,
                record_gate,
                advance_run,
                sync_run,
                delete_run
            ],
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
                    response.set_header(rocket::http::Header::new(
                        "X-Content-Type-Options",
                        "nosniff",
                    ));
                })
            },
        )))
}

/// Makes this dashboard findable once its port is bound: stops with `AlreadyRunning` when another live dashboard
/// is in the data folder, otherwise writes the CLI token and then `dashboard.json`. Returns why it stopped, if it did.
fn announce(runs: &Path, token: &str, port: u16) -> Option<LiftoffStop> {
    let dir = data_dir(runs);
    if crate::discovery::running(dir).is_some() {
        return Some(LiftoffStop::AlreadyRunning);
    }
    let token_file = token_path(runs);
    if let Err(error) = write_private_file(&token_file, token) {
        return Some(LiftoffStop::Failed(format!(
            "could not write the CLI token to {}: {error}",
            token_file.display()
        )));
    }
    let entry = crate::discovery::Dashboard {
        port,
        pid: std::process::id(),
        build: current_build(),
    };
    crate::discovery::write(dir, &entry)
        .err()
        .map(|error| LiftoffStop::Failed(format!("could not write the discovery file: {error}")))
}

/// How often the dashboard server looks for pull requests whose state is due to be read again. Every 20 seconds
/// keeps the dashboard fresh while each pass only reads the pull requests that are due, so hosts are not hammered.
const PR_STATE_PASS: std::time::Duration = std::time::Duration::from_secs(20);

/// Reads the state of the Runs' pull requests from their hosts, one pass every `pass`, on a blocking thread so
/// a slow or missing tool never holds up a request. Merged pull requests are not read again.
fn watch_pr_states(
    store: RunStore,
    tool: std::sync::Arc<dyn pr_wait::Gh + Send + Sync>,
    pass: std::time::Duration,
) -> rocket::tokio::task::JoinHandle<()> {
    rocket::tokio::spawn(async move {
        loop {
            let (store, tool) = (store.clone(), tool.clone());
            let joined = rocket::tokio::task::spawn_blocking(move || {
                let now = match crate::run::updated_now() {
                    Ok(now) => now,
                    Err(error) => return eprintln!("Pull request states were not read: {error}"),
                };
                if let Err(error) = pr_state::refresh(&store, tool.as_ref(), now) {
                    eprintln!("Pull request states were not stored: {error}");
                }
            })
            .await;
            if let Err(error) = joined {
                eprintln!("Pull request states were not read: the read stopped: {error}");
            }
            rocket::tokio::time::sleep(pass).await;
        }
    })
}

/// How often the dashboard server looks in the logs of working runs for a usage limit or compaction. A minute is
/// frequent enough to notice a pause promptly and rare enough that reading the logs stays cheap.
const PAUSE_PASS: std::time::Duration = std::time::Duration::from_secs(60);

/// Collects the runs that are working or paused on their own every `pass`, on a blocking thread, so a
/// pause is seen and cleared without an agent update. One pass is bounded by the collection budget per run.
fn watch_pauses(
    store: RunStore,
    settings: UsageSettings,
    pass: std::time::Duration,
) -> rocket::tokio::task::JoinHandle<()> {
    rocket::tokio::spawn(async move {
        loop {
            rocket::tokio::time::sleep(pass).await;
            let (store, settings) = (store.clone(), settings.clone());
            let joined = rocket::tokio::task::spawn_blocking(move || {
                let result = crate::run::updated_now()
                    .and_then(|now| crate::usage::collect_active(&store, &settings, now));
                if let Err(error) = result {
                    eprintln!("Pauses were not checked: {error}");
                }
            })
            .await;
            if let Err(error) = joined {
                eprintln!("Pauses were not checked: the check stopped: {error}");
            }
        }
    })
}

/// Serves the dashboard on `port` (0 lets the operating system pick one). `url` turns the bound port into the
/// address printed to the user. Returns `Ok` without serving when another dashboard is already running.
pub(crate) fn serve(
    path: &Path,
    port: u16,
    url: impl FnOnce(u16) -> String + Send + Sync + 'static,
    usage: UsageSettings,
) -> io::Result<()> {
    let runtime = rocket::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    runtime.block_on(async {
        let pause_settings = usage.clone();
        // Made before the run-storage step so a token failure keeps its own message.
        let tokens = new_tokens()?;
        let launched = build_rocket_with_tokens(
            path,
            port,
            Collector::new(Some(usage)),
            tokens,
            std::sync::Arc::new(RealGh),
        )
        .map_err(|error| {
            io::Error::new(error.kind(), format!("could not open run storage: {error}"))
        })?
        .attach(rocket::fairing::AdHoc::on_liftoff(
            "Pull request state",
            |rocket| {
                Box::pin(async move {
                    let stopped = rocket
                        .state::<LiftoffStopSlot>()
                        .is_none_or(|slot| slot.0.lock().is_ok_and(|stop| stop.is_some()));
                    if stopped {
                        return;
                    }
                    println!(
                        "Dashboard: {} (press Ctrl+C to stop)",
                        url(rocket.config().port)
                    );
                    if let (Some(store), Some(tool)) =
                        (rocket.state::<RunStore>(), rocket.state::<PrTool>())
                    {
                        watch_pauses(store.clone(), pause_settings, PAUSE_PASS);
                        watch_pr_states(store.clone(), tool.0.clone(), PR_STATE_PASS);
                    }
                })
            },
        ))
        .launch()
        .await
        .map_err(io::Error::other);
        let stop = launched.as_ref().ok().and_then(|rocket| {
            rocket
                .state::<LiftoffStopSlot>()
                .and_then(|slot| slot.0.lock().ok()?.take())
        });
        if matches!(stop, Some(LiftoffStop::AlreadyRunning)) {
            println!("Dashboard already running; this one was shut down.");
            return Ok(());
        }
        // A crash skips this; the stale file then fails the probe and the next start overwrites it.
        crate::discovery::remove_if_ours(
            path.parent().unwrap_or(Path::new(".")),
            std::process::id(),
        );
        launched?;
        match stop {
            Some(LiftoffStop::Failed(message)) => Err(io::Error::other(message)),
            _ => Ok(()),
        }
    })
}

#[cfg(test)]
mod tests;
