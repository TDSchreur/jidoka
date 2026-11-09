use crate::{
    current_build,
    document::{self, DocumentError},
    pr_state,
    pr_wait::{self, RealGh},
    run::{NewRun, Run, RunStore, UpdateRun},
    usage::{collect_and_store, UsageSettings},
};
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
use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub(crate) const DEFAULT_PORT: u16 = 4311;
pub(crate) const ADDRESS: &str = "127.0.0.1:4311";
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
        "help.js",
        ContentType::JavaScript,
        include_bytes!("../dashboard/help.js"),
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
/// The header the `factory` CLI sends its CLI token in.
pub(crate) const CLI_TOKEN_HEADER: &str = "X-Factory-Token";
const BODY_LIMIT: u64 = 64 * 1024;

fn is_local_authority(authority: &str, port: u16) -> bool {
    authority.rsplit_once(':').is_some_and(|(host, given)| {
        matches!(host, "localhost" | "127.0.0.1") && given == port.to_string()
    })
}

/// 32 bytes from the operating system's CSPRNG, as 64 hex characters.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect(
        "cannot start the dashboard: the operating system gave no random bytes for its tokens",
    );
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A random value created when the server starts. Only the dashboard page this server
/// serves to a local host carries it, so a request that has it came from that page.
struct SessionToken(String);

impl SessionToken {
    fn new() -> Self {
        Self(random_token())
    }
}

/// A second random value, per server process, that the `factory` CLI needs to POST. It is
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

/// Where the CLI token lives: next to the runs file, named per port like the runs file.
pub(crate) fn token_path(runs: &Path, port: u16) -> PathBuf {
    let name = if port == DEFAULT_PORT {
        "session-token".to_string()
    } else {
        format!("session-token-{port}")
    };
    runs.with_file_name(name)
}

/// Writes `token` to `path` so no reader sees a partial file; on Unix only the user can read it and the
/// directory is made private.
fn write_token_file(path: &Path, token: &str) -> io::Result<()> {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let first = COUNTER.fetch_add(TOKEN_TEMP_ATTEMPTS, std::sync::atomic::Ordering::Relaxed);
    write_token_file_from(path, token, first)
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

/// `write_token_file`, trying the temporary names from `first_counter` on. A name left by a killed dashboard
/// (same pid, same counter) is skipped; after `TOKEN_TEMP_ATTEMPTS` taken names the error is returned.
fn write_token_file_from(path: &Path, token: &str, first_counter: u32) -> io::Result<()> {
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
        match write_new_file(&temp, token) {
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
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(content.as_bytes())
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

/// What a client needs to recognise a running Factory dashboard and compare its build with its own.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub(crate) struct Info {
    pub(crate) pid: u32,
    pub(crate) version: String,
}

#[rocket::get("/api/info")]
fn info(request: Security) -> Result<Json<Info>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    Ok(Json(Info {
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
    collector.after_update(store, &updated.run.id);
    Ok(Json(updated.run))
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
            run_with_cost(&run).map(Json).map_err(|error| {
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
    let read = rocket::tokio::task::spawn_blocking(move || document::read(&run, &reference))
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
                let _ = rocket::tokio::task::spawn_blocking(move || {
                    if let Err(error) = collect_and_store(&store, &id, &settings) {
                        eprintln!("Usage was not collected for run {id}: {error}");
                    }
                })
                .await;
            }
            if let Ok(mut chains) = chains.lock() {
                if chains.get(&id) == Some(&generation) {
                    chains.remove(&id);
                }
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

#[cfg(test)]
pub(crate) fn build_rocket(path: &Path, port: u16) -> io::Result<Rocket<Build>> {
    build_rocket_with(path, port, Collector::new(None))
}

/// The runs file the CLI token is kept next to.
struct TokenFile(PathBuf);

/// Set at liftoff when the CLI token could not be written; the server shuts down and `serve` reports it.
struct TokenWriteFailure(std::sync::Mutex<Option<String>>);

pub(crate) fn build_rocket_with(
    path: &Path,
    port: u16,
    collector: Collector,
) -> io::Result<Rocket<Build>> {
    let config = Figment::from(rocket::Config::default())
        .merge(("address", "127.0.0.1"))
        .merge(("port", port))
        .merge(("limits", Limits::new().limit("json", BODY_LIMIT.bytes())));
    #[cfg(unix)]
    if let Some(warning) = secure_data_dir(path)? {
        eprintln!("{warning}");
    }
    Ok(rocket::custom(config)
        .manage(RunStore::open(path)?)
        .manage(collector)
        .manage(SessionToken::new())
        .manage(CliToken(random_token()))
        .manage(TokenFile(path.to_path_buf()))
        .manage(TokenWriteFailure(std::sync::Mutex::new(None)))
        .attach(rocket::fairing::AdHoc::on_liftoff(
            "Write CLI token",
            |rocket| {
                Box::pin(async move {
                    // Only once the port is bound, so a dashboard that fails to bind leaves the token alone.
                    if let (Some(token), Some(file), Some(failure)) = (
                        rocket.state::<CliToken>(),
                        rocket.state::<TokenFile>(),
                        rocket.state::<TokenWriteFailure>(),
                    ) {
                        let path = token_path(&file.0, rocket.config().port);
                        if let Err(error) = write_token_file(&path, &token.0) {
                            let message = format!(
                                "could not write the CLI token to {}: {error}",
                                path.display()
                            );
                            eprintln!("{message}");
                            if let Ok(mut slot) = failure.0.lock() {
                                *slot = Some(message);
                            }
                            rocket.shutdown().notify();
                        }
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

/// How often the dashboard server looks for pull requests whose state is due to be read again.
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
            let _ = rocket::tokio::task::spawn_blocking(move || {
                let now = match crate::run::updated_now() {
                    Ok(now) => now,
                    Err(error) => return eprintln!("Pull request states were not read: {error}"),
                };
                if let Err(error) = pr_state::refresh(&store, tool.as_ref(), now) {
                    eprintln!("Pull request states were not stored: {error}");
                }
            })
            .await;
            rocket::tokio::time::sleep(pass).await;
        }
    })
}

/// How often the dashboard server looks in the logs of working runs for a usage limit or compaction.
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

pub(crate) fn serve(path: &Path, port: u16, url: &str, usage: UsageSettings) -> io::Result<()> {
    let runtime = rocket::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    println!("Dashboard: {url} (press Ctrl+C to stop)");
    runtime.block_on(async {
        let pause_settings = usage.clone();
        build_rocket_with(path, port, Collector::new(Some(usage)))
            .map_err(|error| {
                io::Error::new(error.kind(), format!("could not open run storage: {error}"))
            })?
            .attach(rocket::fairing::AdHoc::on_liftoff(
                "Pull request state",
                |rocket| {
                    Box::pin(async move {
                        if let Some(store) = rocket.state::<RunStore>() {
                            watch_pauses(store.clone(), pause_settings, PAUSE_PASS);
                            watch_pr_states(
                                store.clone(),
                                std::sync::Arc::new(RealGh),
                                PR_STATE_PASS,
                            );
                        }
                    })
                },
            ))
            .launch()
            .await
            .map_err(io::Error::other)
            .and_then(|rocket| {
                match rocket
                    .state::<TokenWriteFailure>()
                    .and_then(|failure| failure.0.lock().ok()?.take())
                {
                    Some(message) => Err(io::Error::other(message)),
                    None => Ok(()),
                }
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        run::{load_runs, updated_now, RunStatus, Stage},
        test_support::TestDir,
    };
    use rocket::{http::Header, local::asynchronous::Client};
    use std::fs;

    async fn client(dir: &TestDir) -> Client {
        Client::tracked(build_rocket(&dir.0.path().join("runs.json"), DEFAULT_PORT).unwrap())
            .await
            .unwrap()
    }

    /// A run with the given worktree, repository, spec and ticket references, and its id.
    fn document_run(
        path: &Path,
        worktree: &Path,
        repository: &Path,
        spec: &str,
        tickets: &[&str],
    ) -> String {
        let store = RunStore::open(path).unwrap();
        let run = store
            .create(NewRun {
                title: "t".into(),
                project: repository.to_string_lossy().into_owned(),
                repository: Some(repository.to_string_lossy().into_owned()),
                session: None,
                model_routing: None,
            })
            .unwrap();
        let tickets: Vec<_> = tickets
            .iter()
            .map(|reference| serde_json::json!({ "reference": reference, "title": "T" }))
            .collect();
        store
            .update(
                &run.id,
                serde_json::from_value(serde_json::json!({
                    "stage": "build",
                    "status": "running",
                    "worktree": worktree.to_string_lossy(),
                    "spec": spec,
                    "tickets": tickets,
                }))
                .unwrap(),
            )
            .unwrap();
        run.id
    }

    /// Fetches the document and returns its status and JSON body.
    async fn fetch_document(
        dir: &TestDir,
        id: &str,
        reference: &str,
    ) -> (HttpStatus, serde_json::Value, Option<String>) {
        let api = client(dir).await;
        let response = api
            .get(format!(
                "/api/runs/{id}/document?reference={}",
                percent_encoding::utf8_percent_encode(
                    reference,
                    percent_encoding::NON_ALPHANUMERIC
                )
            ))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await;
        let status = response.status();
        let nosniff = response
            .headers()
            .get_one("X-Content-Type-Options")
            .map(str::to_owned);
        let body = response.into_string().await.unwrap_or_default();
        (
            status,
            serde_json::from_str(&body).unwrap_or_default(),
            nosniff,
        )
    }

    /// A repository and a worktree beside it, and a run recording `spec` and `tickets`.
    fn document_setup(spec: &str, tickets: &[&str]) -> (TestDir, PathBuf, PathBuf, String) {
        let dir = TestDir::new();
        let repository = dir.0.path().join("repo");
        let worktree = dir.0.path().join("repo.worktrees").join("feature");
        fs::create_dir_all(repository.join(".git")).unwrap();
        fs::create_dir_all(worktree.join(".git")).unwrap();
        let id = document_run(
            &dir.0.path().join("runs.json"),
            &worktree,
            &repository,
            spec,
            tickets,
        );
        (dir, repository, worktree, id)
    }

    #[rocket::async_test]
    async fn a_recorded_spec_is_rendered_with_nosniff() {
        let (dir, _, worktree, id) = document_setup("docs/spec.md", &[]);
        fs::create_dir_all(worktree.join("docs")).unwrap();
        fs::write(worktree.join("docs/spec.md"), "# Title\n\nSome *text*").unwrap();
        let (status, body, nosniff) = fetch_document(&dir, &id, "docs/spec.md").await;
        assert_eq!(status, HttpStatus::Ok);
        assert_eq!(body["name"], "spec.md");
        assert!(body["html"].as_str().unwrap().contains("<h1>Title</h1>"));
        assert_eq!(nosniff.as_deref(), Some("nosniff"));
    }

    #[rocket::async_test]
    async fn only_recorded_references_are_served() {
        let (dir, _, worktree, id) = document_setup("spec.md", &["one.md"]);
        fs::write(worktree.join("one.md"), "ticket").unwrap();
        fs::write(worktree.join("other.md"), "other").unwrap();
        let (status, body, _) = fetch_document(&dir, &id, "one.md").await;
        assert_eq!(status, HttpStatus::Ok, "{body}");
        let (status, _, _) = fetch_document(&dir, &id, "other.md").await;
        assert_eq!(status, HttpStatus::Forbidden);
        let (status, _, _) = fetch_document(&dir, "999", "one.md").await;
        assert_eq!(status, HttpStatus::NotFound);
    }

    #[rocket::async_test]
    async fn web_references_are_rejected() {
        let (dir, _, _, id) =
            document_setup("https://example.com/spec.md", &["http://x.test/1.md"]);
        for reference in ["https://example.com/spec.md", "http://x.test/1.md"] {
            let (status, _, _) = fetch_document(&dir, &id, reference).await;
            assert_eq!(status, HttpStatus::BadRequest, "{reference}");
        }
    }

    #[rocket::async_test]
    async fn traversal_out_of_the_roots_is_rejected() {
        let (dir, _, _, id) = document_setup("../../outside.md", &[]);
        fs::write(dir.0.path().join("outside.md"), "secret").unwrap();
        let (status, _, _) = fetch_document(&dir, &id, "../../outside.md").await;
        assert_eq!(status, HttpStatus::Forbidden);
    }

    // Creating symlinks on Windows needs extra privileges.
    #[cfg(unix)]
    #[rocket::async_test]
    async fn a_symlink_out_of_the_roots_is_rejected() {
        let (dir, _, worktree, id) = document_setup("link.md", &[]);
        let outside = dir.0.path().join("outside.md");
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, worktree.join("link.md")).unwrap();
        let (status, _, _) = fetch_document(&dir, &id, "link.md").await;
        assert_eq!(status, HttpStatus::Forbidden);
    }

    #[rocket::async_test]
    async fn only_markdown_files_are_served() {
        let (dir, _, worktree, id) = document_setup("notes.txt", &[]);
        fs::write(worktree.join("notes.txt"), "text").unwrap();
        let (status, _, _) = fetch_document(&dir, &id, "notes.txt").await;
        assert_eq!(status, HttpStatus::BadRequest);
    }

    #[rocket::async_test]
    async fn a_document_over_a_megabyte_is_rejected() {
        let (dir, _, worktree, id) = document_setup("big.md", &[]);
        fs::write(worktree.join("big.md"), vec![b'a'; 1024 * 1024 + 1]).unwrap();
        let (status, _, _) = fetch_document(&dir, &id, "big.md").await;
        assert_eq!(status, HttpStatus::PayloadTooLarge);
    }

    #[rocket::async_test]
    async fn a_missing_file_is_not_available_rather_than_rejected() {
        let (dir, _, _, id) = document_setup("gone.md", &[]);
        let (status, body, _) = fetch_document(&dir, &id, "gone.md").await;
        assert_eq!(status, HttpStatus::NotFound);
        assert_eq!(body["available"], false);
        assert_eq!(body["path"], "gone.md");
    }

    #[rocket::async_test]
    async fn relative_references_resolve_against_the_worktree_then_the_repository() {
        let (dir, repository, worktree, id) = document_setup("spec.md", &["only-repo.md"]);
        fs::write(repository.join("spec.md"), "from repository").unwrap();
        fs::write(worktree.join("spec.md"), "from worktree").unwrap();
        fs::write(repository.join("only-repo.md"), "repo only").unwrap();
        let (_, body, _) = fetch_document(&dir, &id, "spec.md").await;
        assert!(body["html"].as_str().unwrap().contains("from worktree"));
        let (_, body, _) = fetch_document(&dir, &id, "only-repo.md").await;
        assert!(body["html"].as_str().unwrap().contains("repo only"));
    }

    #[rocket::async_test]
    async fn the_saved_plan_beside_the_worktree_is_served() {
        let (dir, _, worktree, _) = document_setup("x.md", &[]);
        let plan = format!("{}.factory-plan.md", worktree.display());
        fs::write(&plan, "plan").unwrap();
        let id = document_run(
            &dir.0.path().join("runs.json"),
            &worktree,
            &dir.0.path().join("repo"),
            &plan,
            &[],
        );
        let (status, body, _) = fetch_document(&dir, &id, &plan).await;
        assert_eq!(status, HttpStatus::Ok, "{body}");
    }

    #[rocket::async_test]
    async fn dashboard_page_shows_which_build_is_running() {
        let page = dashboard_page().await;
        assert!(
            !page.contains(BUILD_PLACEHOLDER),
            "placeholder left in page"
        );
        assert!(page.contains(&format!("<code>{}</code>", current_build())));
    }

    #[rocket::async_test]
    async fn dashboard_page_shows_stage_groups_last_updated_and_rework() {
        let page = dashboard_page().await;
        for group in ["Define", "Build", "Check", "Finish", "Done"] {
            assert!(
                page.contains(&format!("'{group}'")),
                "missing group {group}"
            );
        }
        assert!(page.contains("Needs you"), "missing Needs you label");
        assert!(
            page.contains("runs need you"),
            "missing the andon strip's count"
        );
        assert!(
            page.contains("aria-pressed"),
            "the toggle reports its state"
        );
        assert!(page.contains("document.title"), "the count is in the title");
        assert!(page.contains("m ago"), "missing relative time");
        assert!(
            page.contains("title=${localDateTime(date)}"),
            "exact time tooltip"
        );
        assert!(page.contains("Rework"), "missing Rework label");
        assert!(page.contains("past the cap of"));
    }

    /// The page with the stylesheet and script it loads, so tests can look for any of their text.
    async fn dashboard_page() -> String {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let mut page = String::new();
        for path in ["/", "/assets/style.css", "/assets/app.js"] {
            page += &api
                .get(path)
                .header(Header::new("Host", "127.0.0.1:4311"))
                .dispatch()
                .await
                .into_string()
                .await
                .unwrap();
        }
        page
    }

    #[rocket::async_test]
    async fn assets_are_served_with_their_content_types() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        for (path, content_type) in [
            ("/assets/app.js", ContentType::JavaScript),
            ("/assets/help.js", ContentType::JavaScript),
            ("/assets/favicon.svg", ContentType::SVG),
            ("/assets/style.css", ContentType::CSS),
            ("/assets/vendor/preact.js", ContentType::JavaScript),
            ("/assets/vendor/hooks.js", ContentType::JavaScript),
            ("/assets/vendor/htm.js", ContentType::JavaScript),
        ] {
            let response = api
                .get(path)
                .header(Header::new("Host", "127.0.0.1:4311"))
                .dispatch()
                .await;
            assert_eq!(response.status(), HttpStatus::Ok, "{path}");
            assert_eq!(response.content_type(), Some(content_type), "{path}");
            assert_eq!(
                response.headers().get_one("Cache-Control"),
                Some("no-store")
            );
        }
    }

    #[rocket::async_test]
    async fn only_listed_assets_are_served() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        for path in [
            "/assets/missing.js",
            "/assets/",
            "/assets/vendor",
            "/assets/../Cargo.toml",
            "/assets/%2e%2e/Cargo.toml",
            "/assets/app.js/extra",
        ] {
            let response = api
                .get(path)
                .header(Header::new("Host", "127.0.0.1:4311"))
                .dispatch()
                .await;
            assert_eq!(response.status(), HttpStatus::NotFound, "{path}");
        }
    }

    #[rocket::async_test]
    async fn assets_reject_a_foreign_host() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let response = api
            .get("/assets/app.js")
            .header(Header::new("Host", "example.com"))
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::BadRequest);
    }

    #[rocket::async_test]
    async fn dashboard_page_loads_its_stylesheet_and_script_from_assets() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let page = api
            .get("/")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap();
        assert!(page.contains("href=\"/assets/style.css\""));
        assert!(page.contains("src=\"/assets/app.js\""));
    }

    #[rocket::async_test]
    async fn import_map_names_only_served_vendor_files() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let page = api
            .get("/")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap();
        let map = &page[page.find("importmap").unwrap()..];
        let map = &map[..map.find("</script>").unwrap()];
        let mut mapped = 0;
        for target in map.split('"').filter(|part| part.starts_with("/assets/")) {
            mapped += 1;
            let response = api
                .get(target.to_owned())
                .header(Header::new("Host", "127.0.0.1:4311"))
                .dispatch()
                .await;
            assert_eq!(response.status(), HttpStatus::Ok, "{target}");
        }
        assert_eq!(mapped, 3, "preact, preact/hooks and htm");
    }

    /// Custom property names declared in the CSS block that opens at `marker`.
    fn tokens_after(page: &str, marker: &str) -> std::collections::BTreeSet<String> {
        let start = page
            .find(marker)
            .unwrap_or_else(|| panic!("missing {marker}"));
        let block = &page[start..page[start..].find('}').map(|i| start + i).unwrap()];
        block
            .split(&[';', '{'][..])
            .filter_map(|decl| decl.trim_start().strip_prefix("--"))
            .filter_map(|decl| decl.split(':').next())
            .map(str::to_owned)
            .collect()
    }

    /// A served asset as text, with line endings normalised to `\n`: a Windows checkout has CRLF, and tests cut
    /// components out of the source at `\n}\n`.
    async fn asset(path: &'static str) -> String {
        let dir = TestDir::new();
        client(&dir)
            .await
            .get(path)
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap()
            .replace("\r\n", "\n")
    }

    async fn stylesheet() -> String {
        asset("/assets/style.css").await
    }

    /// The design token layer and the rules below it.
    fn split_tokens(css: &str) -> (&str, &str) {
        css.split_once("/* End of design tokens */")
            .expect("the stylesheet ends its token layer with a marker")
    }

    fn variable_uses(rules: &str) -> std::collections::BTreeSet<String> {
        rules
            .split("var(--")
            .skip(1)
            .filter_map(|rest| rest.split(')').next())
            .map(str::to_owned)
            .collect()
    }

    #[rocket::async_test]
    async fn dashboard_theme_follows_system_and_dark_overrides_every_themed_token() {
        let css = stylesheet().await;
        // Single-line markers keep the test independent of line endings (CRLF on Windows).
        // The first `:root {` block holds the light tokens.
        let light = tokens_after(&css, ":root {");
        assert!(light.len() > 10, "design values should be tokens on :root");
        assert!(css.contains("color-scheme: light dark;"));
        assert!(css.contains("@media (prefers-color-scheme: dark)"));
        // Colours and shadows change with the theme; the rest (spacing, type) is shared.
        let themed: std::collections::BTreeSet<_> = light
            .iter()
            .filter(|name| name.starts_with("color") || name.starts_with("shadow"))
            .cloned()
            .collect();
        assert!(!themed.is_empty());
        let system_dark = tokens_after(&css, ":root:not([data-theme=\"light\"]) {");
        let forced_dark = tokens_after(&css, ":root[data-theme=\"dark\"] {");
        assert_eq!(system_dark, themed, "system dark theme misses tokens");
        assert_eq!(forced_dark, themed, "forced dark theme misses tokens");
        assert!(css.contains(":root[data-theme=\"light\"] { color-scheme: light; }"));
    }

    #[rocket::async_test]
    async fn dashboard_rules_use_tokens_and_no_literal_colour() {
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        // Only declaration bodies can hold a colour; selectors such as `#add` are not colours.
        let declarations: String = rules
            .split('}')
            .filter_map(|rule| rule.split_once('{').map(|(_, body)| body))
            .collect::<Vec<_>>()
            .join(";");
        for (at, _) in declarations.match_indices('#') {
            let hex = declarations[at + 1..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .count();
            assert!(
                !matches!(hex, 3 | 4 | 6 | 8),
                "literal colour near {:?}",
                &declarations[at..(at + 12).min(declarations.len())]
            );
        }
        for function in [
            "rgb(",
            "rgba(",
            "hsl(",
            "hsla(",
            "hwb(",
            "lab(",
            "lch(",
            "oklab(",
            "oklch(",
            "color(",
            "color-mix(",
            "light-dark(",
        ] {
            assert!(
                !declarations.contains(function),
                "literal colour {function}"
            );
        }
        let words: std::collections::BTreeSet<_> = declarations
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .map(str::to_ascii_lowercase)
            .collect();
        for named in [
            "white",
            "black",
            "red",
            "green",
            "blue",
            "gray",
            "grey",
            "orange",
            "yellow",
            "purple",
            "navy",
            "teal",
            "pink",
            "transparent",
            "currentcolor",
            "canvas",
            "canvastext",
            "linktext",
            "highlight",
        ] {
            assert!(!words.contains(named), "named colour {named}");
        }
    }

    #[rocket::async_test]
    async fn selecting_a_card_opens_a_modal_detail_with_every_run_field() {
        let script = asset("/assets/app.js").await;
        // The card's title is the button that opens the detail, found again by the Run id.
        assert!(script.contains("class=\"line-open\""));
        assert!(script.contains("data-run-key="));
        // A native modal dialog named by the Run title.
        assert!(script.contains("<dialog"));
        assert!(script.contains("showModal()"));
        assert!(script.contains("aria-labelledby=\"detail-title\""));
        assert!(script.contains("id=\"detail-title\""));
        // The sheet is a route card: a status row, the four stations, token usage and a footer with the ids.
        for part in [
            "<ol class=\"route\" aria-label=\"Route\">",
            "<div class=${blocked ? 'status-row blocked' : 'status-row'}>",
            "<details class=\"usage\"",
            "<div class=\"detail-foot\">",
            "<${CopyButton} value=${run.branch} what=\"branch\" />",
            "<${CopyButton} value=${run.repository} what=\"repository\" />",
            "<${CopyButton} value=${String(run.id)} what=\"run id\" />",
            "Skills and activity are reported by the agent and may be incomplete.",
        ] {
            assert!(script.contains(part), "missing {part}");
        }
        // A field the Run never recorded is left out, not shown as unknown.
        assert!(!script.contains("Unknown\""));
        assert!(!script.contains("Not set"));
        assert!(!script.contains("Worktree") && !script.contains("base_commit"));
    }

    #[rocket::async_test]
    async fn every_delete_asks_first_and_sends_the_session_token() {
        let script = asset("/assets/app.js").await;
        // Delete in a Run's detail and Clear shipped both open a confirmation dialog first.
        assert!(script.contains("onDelete=${(run) => setPending({ runs: [run] })}"));
        assert!(script.contains("onClear=${() => setPending({ shipped: true, runs: shipped.map((entry) => entry.run) })}"));
        assert!(script.contains("<${Confirm}"));
        assert!(script.contains("aria-labelledby=\"confirm-title\""));
        assert!(script.contains("autofocus"), "Cancel takes focus first");
        // Only the confirm button sends the request, with the page's session token.
        assert!(script.contains("method: 'DELETE'"));
        assert!(script.contains("'X-Session-Token': sessionToken()"));
        assert_eq!(
            script.matches("deleteRun(").count(),
            2,
            "one definition, one use"
        );
        assert!(script
            .contains("The repository, worktrees, branches and pull requests stay as they are."));
        // No row on the board has a Delete of its own.
        assert!(!script.contains("now=${now} onOpen=${onOpen} onDelete=${onDelete} />"));
    }

    #[rocket::async_test]
    async fn welcome_card_and_help_dialog_use_the_glossary_terms() {
        let script = asset("/assets/app.js").await;
        // The welcome card replaces the board when there are no Runs, and links to Help.
        assert!(script.contains("runs && !runs.length ? html`<${Welcome}"));
        assert!(script.contains("visibleRuns?.length"));
        assert!(script.contains("Read the Help"));
        // Help opens from the header. It is a native modal named by its heading, with tabs, and the selected
        // tab takes focus. It lives in its own module.
        assert!(script.contains("class=\"toggle help-open\""));
        let help = asset("/assets/help.js").await;
        assert!(help.contains("aria-labelledby=\"help-title\""));
        assert!(help.contains("role=\"tablist\""));
        assert!(help.contains("autofocus=${id === current}"));
        for term in ["Stages", "Needs you", "Rework"] {
            assert!(
                help.contains(&format!("title=\"{term}\"")) || help.contains(&format!("'{term}'")),
                "missing {term}"
            );
        }
        let glossary = include_str!("../GLOSSARY.md");
        for term in ["Run", "Stage", "Needs you", "Rework"] {
            assert!(
                glossary.contains(&format!("**{term}**")),
                "{term} is not in the glossary"
            );
        }
    }

    #[rocket::async_test]
    async fn cards_leave_project_names_in_the_swimlane_and_run_details() {
        let script = asset("/assets/app.js").await;
        assert!(script.contains("function projectNames(runs)"));
        assert!(script.contains("title=${path}>${name}"));
        assert!(
            !script.contains("<p class=\"project\"><${ProjectName} path=${projectOf(run)} /></p>")
        );
        assert!(script.contains("><${ProjectName} path=${project} />"));
        assert!(script.contains("<span><${ProjectName} path=${projectOf(run)} /></span>"));
    }

    #[rocket::async_test]
    async fn selection_and_notifications_are_keyed_per_project_and_run() {
        let script = asset("/assets/app.js").await;
        assert!(script.contains("const runKey = (run) =>"));
        assert!(script.contains("known.current.has(runKey(run))"));
        assert!(script.contains("runKey(run) === openKey"));
        // Deleting a Project takes only that Project's Runs.
        assert!(script.contains("runs.filter((run) => projectOf(run) === project)"));
    }

    #[rocket::async_test]
    async fn info_names_the_process_and_build_and_needs_a_local_host() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let response = api
            .get("/api/info")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::Ok);
        let info: Info = response.into_json().await.unwrap();
        assert_eq!(info.pid, std::process::id());
        assert_eq!(info.version, current_build());
        let foreign = api
            .get("/api/info")
            .header(Header::new("Host", "attacker.example"))
            .dispatch()
            .await;
        assert_eq!(foreign.status(), HttpStatus::BadRequest);
    }

    #[test]
    fn local_authority_follows_the_configured_port() {
        assert!(is_local_authority("127.0.0.1:4400", 4400));
        assert!(is_local_authority("localhost:4400", 4400));
        assert!(!is_local_authority("127.0.0.1:4311", 4400));
        assert!(!is_local_authority("example.com:4400", 4400));
        assert!(!is_local_authority("127.0.0.1", 4311));
        assert!(is_local_origin("http://localhost:4400", 4400));
        assert!(!is_local_origin("http://localhost:4311", 4400));
    }

    async fn start(api: &Client, body: &str) -> Run {
        let response = api
            .post("/api/runs")
            .header(cli_token_header(api))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(body)
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::Created);
        response.into_json().await.unwrap()
    }

    async fn fetch(api: &Client, id: &str) -> (HttpStatus, Option<Run>) {
        let response = api
            .get(format!("/api/runs/{id}"))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await;
        (response.status(), response.into_json().await)
    }

    /// A dashboard whose usage collection reads fixture logs under `home` instead of a user's own.
    async fn client_collecting(dir: &TestDir, home: &Path, enabled: bool) -> Client {
        let roots = crate::usage_logs::Roots {
            claude: vec![home.join("claude/projects")],
            codex: home.join("codex/sessions"),
            copilot: home.join("copilot/session-state"),
        };
        let settings = UsageSettings {
            enabled,
            roots,
            budget: std::time::Duration::from_secs(10),
            settle: Vec::new(),
        };
        Client::tracked(
            build_rocket_with(
                &dir.0.path().join("runs.json"),
                DEFAULT_PORT,
                Collector::new(Some(settings)),
            )
            .unwrap(),
        )
        .await
        .unwrap()
    }

    /// A Claude transcript for a run: one line naming the run, one response at `ts` with `input` tokens.
    fn claude_fixture(
        home: &Path,
        session: &str,
        run_id: &str,
        response: &str,
        ts: u64,
        input: u64,
    ) {
        let line = serde_json::json!({"type": "assistant", "timestamp": crate::usage::iso_timestamp(ts), "requestId": response,
            "message": {"id": response, "model": "claude-test", "usage": {"input_tokens": input, "output_tokens": 1}}});
        let mention = serde_json::json!({"type": "user", "message": {"content": format!("factory run update {run_id}")}});
        let path = home
            .join("claude/projects/-w")
            .join(format!("{session}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("{mention}\n{line}\n")).unwrap();
    }

    /// When the run's timeline began: a log entry at that moment is inside the run's window.
    async fn began(api: &Client, run: &Run) -> u64 {
        fetch(api, &run.id).await.1.unwrap().timeline[0].at
    }

    #[rocket::async_test]
    async fn usage_is_collected_in_the_background_when_a_run_moves_on() {
        let dir = TestDir::new();
        let home = tempfile::tempdir().unwrap();
        let api = client_collecting(&dir, home.path(), true).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        claude_fixture(
            home.path(),
            "session-one",
            &run.id,
            "r1",
            began(&api, &run).await,
            42,
        );
        let collector = api.rocket().state::<Collector>().unwrap();

        // Every update collects, even one that changes neither the stage nor the ticket.
        assert_eq!(
            report(
                &api,
                &run.id,
                r#"{"stage":"define","status":"running","activity":"x"}"#
            )
            .await
            .status(),
            HttpStatus::Ok
        );
        collector.wait_idle().await;
        let usage = fetch(&api, &run.id).await.1.unwrap().usage;
        assert_eq!(
            usage.expect("a plain update collects").sessions,
            ["session-one"]
        );

        // Moving to Build collects, without delaying the answer, and records what it found.
        let store = api.rocket().state::<RunStore>().unwrap().clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let settings = UsageSettings {
                enabled: false,
                roots: crate::usage_logs::Roots {
                    claude: Default::default(),
                    codex: Default::default(),
                    copilot: Default::default(),
                },
                budget: std::time::Duration::from_secs(5),
                settle: Vec::new(),
            };
            store
                .usage_collection()
                .collect_paused(&store, "missing", &settings, None, ready_tx, release_rx)
                .unwrap();
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let response = rocket::tokio::time::timeout(
            std::time::Duration::from_secs(5),
            report(&api, &run.id, r#"{"stage":"build","status":"running"}"#),
        )
        .await;
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        let response = response.expect("update should respond while collection is blocked");
        assert_eq!(response.status(), HttpStatus::Ok);
        collector.wait_idle().await;
        let usage = fetch(&api, &run.id)
            .await
            .1
            .unwrap()
            .usage
            .expect("collected");
        assert_eq!(usage.sessions, ["session-one"]);
        assert_eq!(usage.rows[0].tokens.input, Some(42));

        // Collecting again after finishing gives the same numbers, not double.
        report(
            &api,
            &run.id,
            r#"{"stage":"finish","status":"done","summaries":{"finish":"Shipped"}}"#,
        )
        .await;
        collector.wait_idle().await;
        let usage = fetch(&api, &run.id).await.1.unwrap().usage.unwrap();
        assert_eq!(
            usage
                .rows
                .iter()
                .filter_map(|r| r.tokens.input)
                .sum::<u64>(),
            42
        );
    }

    #[rocket::async_test]
    async fn finish_done_without_a_summary_is_a_bad_request() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        let response = report(&api, &run.id, r#"{"stage":"finish","status":"done"}"#).await;
        assert_eq!(response.status(), HttpStatus::BadRequest);
        assert!(response
            .into_string()
            .await
            .unwrap()
            .contains("--summary finish="));
        let stored = fetch(&api, &run.id).await.1.unwrap();
        assert!(stored.finished_at.is_none());
    }

    #[rocket::async_test]
    async fn a_late_pass_picks_up_what_the_agent_wrote_after_the_update() {
        let dir = TestDir::new();
        let home = tempfile::tempdir().unwrap();
        let roots = crate::usage_logs::Roots {
            claude: vec![home.path().join("claude/projects")],
            codex: home.path().join("codex/sessions"),
            copilot: home.path().join("copilot/session-state"),
        };
        let settings = UsageSettings {
            enabled: true,
            roots,
            budget: std::time::Duration::from_secs(10),
            settle: vec![std::time::Duration::ZERO, std::time::Duration::ZERO],
        };
        let api = Client::tracked(
            build_rocket_with(
                &dir.0.path().join("runs.json"),
                DEFAULT_PORT,
                Collector::new(Some(settings)),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        claude_fixture(
            home.path(),
            "session-one",
            &run.id,
            "r1",
            began(&api, &run).await,
            5,
        );
        report(&api, &run.id, r#"{"stage":"build","status":"running"}"#).await;
        let collector = api.rocket().state::<Collector>().unwrap();
        collector.wait_idle().await;
        // One immediate pass and one per delay; the answer did not wait for any of them.
        assert_eq!(
            collector.passes.load(std::sync::atomic::Ordering::SeqCst),
            3
        );
        // Reading the same logs again changes nothing.
        let usage = fetch(&api, &run.id).await.1.unwrap().usage.unwrap();
        assert_eq!(
            usage
                .rows
                .iter()
                .filter_map(|r| r.tokens.input)
                .sum::<u64>(),
            5
        );
    }

    #[rocket::async_test]
    async fn switching_collection_off_records_why_and_reads_no_logs() {
        let dir = TestDir::new();
        let home = tempfile::tempdir().unwrap();
        let api = client_collecting(&dir, home.path(), false).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        claude_fixture(
            home.path(),
            "session-one",
            &run.id,
            "r1",
            began(&api, &run).await,
            42,
        );
        report(&api, &run.id, r#"{"stage":"build","status":"running"}"#).await;
        api.rocket().state::<Collector>().unwrap().wait_idle().await;
        let usage = fetch(&api, &run.id).await.1.unwrap().usage.unwrap();
        assert_eq!(usage.status, crate::usage::UsageStatus::NotTracked);
        assert!(usage.reason.unwrap().contains("switched off"));
        assert!(usage.rows.is_empty());
    }

    #[rocket::async_test]
    async fn a_collection_that_fails_never_fails_or_changes_the_update() {
        let dir = TestDir::new();
        let home = tempfile::tempdir().unwrap();
        let api = client_collecting(&dir, home.path(), true).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        claude_fixture(
            home.path(),
            "session-one",
            &run.id,
            "r1",
            began(&api, &run).await,
            7,
        );
        let store = api.rocket().state::<RunStore>().unwrap();
        let path = dir.0.path().join("runs.json");
        // The update itself saves first; then storing the usage fails, because the temporary file is blocked.
        let response = report(&api, &run.id, r#"{"stage":"build","status":"running"}"#).await;
        assert_eq!(response.status(), HttpStatus::Ok);
        api.rocket().state::<Collector>().unwrap().wait_idle().await;
        let before = fs::read(&path).unwrap();
        fs::create_dir(path.with_extension("json.tmp")).unwrap();
        let settings = UsageSettings {
            enabled: true,
            roots: crate::usage_logs::Roots {
                claude: vec![home.path().join("claude/projects")],
                codex: home.path().join("codex/sessions"),
                copilot: home.path().join("copilot/session-state"),
            },
            budget: std::time::Duration::from_secs(10),
            settle: Vec::new(),
        };
        claude_fixture(
            home.path(),
            "session-two",
            &run.id,
            "r2",
            began(&api, &run).await,
            9,
        );
        assert!(collect_and_store(store, &run.id, &settings).is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "stored usage is unchanged"
        );
        fs::remove_dir(path.with_extension("json.tmp")).unwrap();
        collect_and_store(store, &run.id, &settings).unwrap();
        let usage = fetch(&api, &run.id).await.1.unwrap().usage.unwrap();
        assert_eq!(
            usage
                .rows
                .iter()
                .filter_map(|r| r.tokens.input)
                .sum::<u64>(),
            16
        );
    }

    #[rocket::async_test]
    async fn counts_survive_a_log_that_disappears_and_a_restart() {
        let dir = TestDir::new();
        let home = tempfile::tempdir().unwrap();
        let api = client_collecting(&dir, home.path(), true).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        claude_fixture(
            home.path(),
            "session-one",
            &run.id,
            "r1",
            began(&api, &run).await,
            42,
        );
        report(&api, &run.id, r#"{"stage":"build","status":"running"}"#).await;
        api.rocket().state::<Collector>().unwrap().wait_idle().await;
        // The vendor cleans up its logs; the next collection finds nothing but must not wipe the counts.
        fs::remove_dir_all(home.path().join("claude")).unwrap();
        report(&api, &run.id, r#"{"stage":"check","status":"running"}"#).await;
        api.rocket().state::<Collector>().unwrap().wait_idle().await;
        drop(api);
        let restarted = client_collecting(&dir, home.path(), true).await;
        let usage = fetch(&restarted, &run.id).await.1.unwrap().usage.unwrap();
        assert_eq!(usage.rows[0].tokens.input, Some(42));
        assert_eq!(usage.status, crate::usage::UsageStatus::Partial);
        assert!(
            usage
                .notes
                .iter()
                .any(|n| n.contains("no longer available")),
            "{usage:?}"
        );
    }

    #[rocket::async_test]
    async fn run_context_goes_through_the_api_partially_and_is_readable_by_id() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let run = start(
            &api,
            r#"{"title":"t","project":"/work/repo.worktrees/x","repository":"/work/repo"}"#,
        )
        .await;
        assert_eq!(run.repository.as_deref(), Some("/work/repo"));
        // A client that does not know about context still starts runs.
        let plain = start(&api, r#"{"title":"old client","project":"/work"}"#).await;
        assert!(plain.repository.is_none());

        let body = r#"{"stage":"build","status":"running","mode":"factory","branch":"feature/x",
            "tickets":[{"reference":"https://x.test/1","title":"One"},{"reference":"docs/2.md","title":"Two"}],
            "ticket_status":{"https://x.test/1":"in-progress"},"summaries":{"define":"Planned"}}"#;
        assert_eq!(report(&api, &run.id, body).await.status(), HttpStatus::Ok);
        // Only the fields that are sent change; null clears one.
        let body =
            r#"{"stage":"build","status":"running","branch":null,"summaries":{"build":"Started"}}"#;
        assert_eq!(report(&api, &run.id, body).await.status(), HttpStatus::Ok);

        let (status, stored) = fetch(&api, &run.id).await;
        assert_eq!(status, HttpStatus::Ok);
        let stored = stored.unwrap();
        assert_eq!(stored.mode, Some(crate::run::Mode::Full));
        assert!(stored.branch.is_none());
        assert_eq!(stored.tickets.len(), 2);
        assert_eq!(
            stored.tickets[0].status,
            crate::run::TicketStatus::InProgress
        );
        assert_eq!(stored.summaries.len(), 2);
        // The other run is untouched.
        assert!(fetch(&api, &plain.id).await.1.unwrap().tickets.is_empty());
        assert_eq!(fetch(&api, "999").await.0, HttpStatus::NotFound);
        assert_eq!(fetch(&api, "12a").await.0, HttpStatus::BadRequest);
    }

    #[rocket::async_test]
    async fn linked_worktrees_group_under_their_repository_and_cards_show_progress_and_links() {
        let script = asset("/assets/app.js").await;
        // Rows, names, keys and notifications all use the repository, falling back to where the run started.
        assert!(script.contains("const projectOf = (run) => run.repository ?? run.project;"));
        assert!(script.contains("const runKey = (run) => `${projectOf(run)}\\n${run.id}`;"));
        assert!(
            !script.contains("entry.run.project"),
            "grouping never reads the raw path"
        );
        // Cards: a ticket-progress chip and http(s)-only links to the pull request and spec.
        assert!(script.contains("Tickets${' '}${progress.done}/${progress.total}"));
        assert!(script.contains("url.protocol === 'http:' || url.protocol === 'https:'"));
        assert!(script.contains("named('Pull request')"));
        // Detail lists the tickets in order with their status, and the summaries per Stage.
        assert!(script.contains("tickets.map((ticket) =>"));
        assert!(script.contains("run.summaries?.[id]"));
    }

    #[rocket::async_test]
    async fn a_run_recorded_before_context_is_served_with_everything_unknown() {
        let dir = TestDir::new();
        fs::write(
            dir.0.path().join("runs.json"),
            r#"[{"id":"7","title":"old","project":"/work/old","stage":"build","status":"running"}]"#,
        )
        .unwrap();
        let api = client(&dir).await;
        let (status, run) = fetch(&api, "7").await;
        assert_eq!(status, HttpStatus::Ok);
        let run = run.unwrap();
        assert!(run.repository.is_none() && run.worktree.is_none() && run.mode.is_none());
        let list: Vec<Run> = api
            .get("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_json()
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        // An update adds context to it without inventing the rest.
        let response = report(
            &api,
            "7",
            r#"{"stage":"build","status":"running","branch":"feature/x"}"#,
        )
        .await;
        assert_eq!(response.status(), HttpStatus::Ok);
        let run = fetch(&api, "7").await.1.unwrap();
        assert_eq!(run.branch.as_deref(), Some("feature/x"));
        assert!(run.repository.is_none() && run.tickets.is_empty());
    }

    #[rocket::async_test]
    async fn starting_a_run_validates_its_model_routing_selection() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let with = |routing: &str| {
            format!(r#"{{"title":"t","project":"/work","model_routing":{routing}}}"#)
        };
        let long = "m".repeat(65);
        for routing in [
            // The earlier shape, with roles and tasks.
            r#"{"agent":"claude","roles":{"strong":{"model":"sonnet"}},"tasks":{"check":"strong"}}"#.to_string(),
            // Empty, oversized and unsafe text.
            r#"{"agent":"claude","stages":{"check":{"model":""}}}"#.to_string(),
            format!(r#"{{"agent":"claude","stages":{{"check":{{"model":"{long}"}}}}}}"#),
            r#"{"agent":"claude","stages":{"check":{"model":"son net"}}}"#.to_string(),
            r#"{"agent":"claude","stages":{"check":{"model":"sonnet","effort":"hi\u001b[2Jgh"}}}"#.to_string(),
            // Unknown field, unknown stage, unknown agent.
            r#"{"agent":"claude","stages":{},"extra":1}"#.to_string(),
            r#"{"agent":"claude","stages":{"deploy":{"model":"sonnet"}}}"#.to_string(),
            r#"{"agent":"gemini","stages":{}}"#.to_string(),
        ] {
            let response = api
                .post("/api/runs")
                .header(cli_token_header(&api))
                .header(Header::new("Host", "127.0.0.1:4311"))
                .header(ContentType::JSON)
                .body(with(&routing))
                .dispatch()
                .await;
            assert_eq!(response.status(), HttpStatus::BadRequest, "{routing}");
        }
        assert!(api
            .rocket()
            .state::<RunStore>()
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
        let run = start(
            &api,
            &with(
                r#"{"agent":"codex","stages":{"check":{"model":"gpt-6-astra","effort":"high"}}}"#,
            ),
        )
        .await;
        let routing = run.model_routing.unwrap();
        assert_eq!(routing.stages.len(), 1);
    }

    #[rocket::async_test]
    async fn starting_a_run_validates_its_identity_fields() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        for body in [
            r#"{"title":"t","project":"/work","repository":"relative/repo"}"#,
            r#"{"title":"t","project":"/work","repository":"/work/\u001b[2J"}"#,
            r#"{"title":"bad\u0007title","project":"/work"}"#,
        ] {
            let response = api
                .post("/api/runs")
                .header(cli_token_header(&api))
                .header(Header::new("Host", "127.0.0.1:4311"))
                .header(ContentType::JSON)
                .body(body)
                .dispatch()
                .await;
            assert_eq!(response.status(), HttpStatus::BadRequest, "{body}");
        }
        assert!(api
            .rocket()
            .state::<RunStore>()
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
        let run = start(
            &api,
            r#"{"title":"t","project":"C:\\work\\repo","repository":"C:\\work\\repo"}"#,
        )
        .await;
        assert_eq!(run.repository.as_deref(), Some("C:\\work\\repo"));
    }

    #[rocket::async_test]
    async fn cards_show_fresh_tokens_and_run_detail_the_breakdown_without_inventing_zeros() {
        let script = asset("/assets/app.js").await;
        let css = asset("/assets/style.css").await;
        // The card chip: fresh tokens, marked partial, and none at all when nothing is tracked.
        assert!(script
            .contains("const freshTokens = (t) => (t.input == null && t.output == null ? null :"));
        assert!(script.contains("if (!usage || usage.status === 'not-tracked'"));
        assert!(script.contains("run.usage.status === 'partial' ? ' (partial)' : ''"));
        assert!(script.contains("tokens !== null"));
        // The detail: a labelled, keyboard-scrollable table by stage, model and ticket, with n/a for unreported fields.
        for header in [
            "Stage",
            "Model",
            "Ticket",
            "Fresh",
            "Input",
            "Cache read",
            "Cache write",
            "Output",
            "Reasoning",
            "Subagents",
        ] {
            assert!(
                script.contains(&format!("<th scope=\"col\">{header}</th>")),
                "{header}"
            );
        }
        assert!(
            script.contains("aria-label=\"Token usage by stage, model and ticket\" tabindex=\"0\"")
        );
        assert!(script.contains("n === undefined || n === null ? 'n/a'"));
        assert!(script.contains("<details class=\"usage\" open=${active}>"));
        assert!(script.contains(
            "Fresh tokens are input excluding cache reads, plus cache writes and output."
        ));
        assert!(script.contains("No data for:"));
        assert!(css.contains(".usage-table {") && css.contains(".table-scroll:focus-visible"));
        // The cost comes from the server; the dashboard holds no prices of its own.
        assert!(script.contains("usd === null || usd === undefined) return 'unknown'"));
        assert!(!script.contains("claude-opus") && !script.contains("per million"));
    }

    #[rocket::async_test]
    async fn inconsistent_context_updates_get_a_400_that_says_why() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        for (body, reason) in [
            (
                r#"{"stage":"build","status":"running","worktree":"relative"}"#,
                "absolute path",
            ),
            (
                r#"{"stage":"build","status":"running","pr":"https://u:p@x.test/1"}"#,
                "credentials",
            ),
            (
                r#"{"stage":"build","status":"running","spec":"ftp://x.test/spec"}"#,
                "HTTP(S) URL or a local path",
            ),
            (
                r#"{"stage":"build","status":"running","tickets":[{"reference":"a","title":"A"},{"reference":"a","title":"B"}]}"#,
                "listed twice",
            ),
            (
                r#"{"stage":"build","status":"running","ticket_status":{"nope":"done"}}"#,
                "No ticket nope",
            ),
            (
                r#"{"stage":"build","status":"running","summaries":{"define":"x\u001b[2J"}}"#,
                "control characters",
            ),
        ] {
            let response = report(&api, &run.id, body).await;
            assert_eq!(response.status(), HttpStatus::BadRequest, "{body}");
            let text = response.into_string().await.unwrap();
            assert!(text.contains(reason), "{body}: {text}");
        }
        // Unknown fields and stages are still refused, and nothing was stored.
        let response = report(
            &api,
            &run.id,
            r#"{"stage":"build","status":"running","transcript":"hi"}"#,
        )
        .await;
        assert_ne!(response.status(), HttpStatus::Ok);
        let stored = fetch(&api, &run.id).await.1.unwrap();
        assert!(stored.worktree.is_none() && stored.pr.is_none() && stored.tickets.is_empty());
    }

    #[rocket::async_test]
    async fn run_reports_accept_valid_updates_and_reject_bad_reports() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        let body = r#"{"stage":"build","status":"running","run_reports":[{"kind":"skill","text":"factory-tdd"}]}"#;
        assert_eq!(report(&api, &run.id, body).await.status(), HttpStatus::Ok);
        let bad =
            r#"{"stage":"build","status":"running","run_reports":[{"kind":"skill","text":""}]}"#;
        assert_eq!(
            report(&api, &run.id, bad).await.status(),
            HttpStatus::BadRequest
        );
        let saved = fetch(&api, &run.id).await.1.unwrap();
        assert_eq!(saved.run_reports.len(), 1);
        assert_eq!(saved.run_reports[0].text, "factory-tdd");
    }

    #[rocket::async_test]
    async fn ticket_reports_are_recorded_and_rejected_atomically_via_api() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
        let body = r#"{"stage":"build","status":"running","tickets":[{"reference":"local=42","title":"One"}],"ticket_reports":[{"ticket":"local=42","kind":"skill","text":"factory-tdd"}]}"#;
        assert_eq!(report(&api, &run.id, body).await.status(), HttpStatus::Ok);
        let bad = r#"{"stage":"build","status":"running","ticket_reports":[{"ticket":"missing","kind":"activity","text":"oops"}]}"#;
        assert_eq!(
            report(&api, &run.id, bad).await.status(),
            HttpStatus::BadRequest
        );
        let saved = fetch(&api, &run.id).await.1.unwrap();
        assert_eq!(saved.ticket_reports.len(), 1);
        assert_eq!(saved.ticket_reports[0].text, "factory-tdd");
    }

    #[rocket::async_test]
    async fn the_header_has_no_dashboard_title_and_a_connection_indicator_follows_the_poll() {
        let dir = TestDir::new();
        let page = client(&dir)
            .await
            .get("/")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap();
        assert!(!page.contains("Factory dashboard"));
        let script = asset("/assets/app.js").await;
        // The indicator is fed by the Run poll and says its state in words, not by colour alone.
        assert!(script.contains("lastOk: Date.now()"));
        for word in ["Offline", "Connecting…", "Connected"] {
            assert!(script.contains(word), "{word}");
        }
        // The dot restarts its pulse when the state changes, not on every poll.
        assert!(script.contains("<span class=\"pulse\" key=${state}"));
        assert!(!script.contains("beat"), "no per-poll counter is left");
    }

    #[rocket::async_test]
    async fn quiet_controls_leave_attention_for_runs_that_need_you() {
        let page = dashboard_page().await;
        let css = stylesheet().await;
        let script = asset("/assets/app.js").await;
        let rule = |selector: &str| {
            css.lines()
                .find(|line| line.starts_with(&format!("{selector} {{")))
                .unwrap_or_else(|| panic!("no {selector} rule"))
                .to_string()
        };
        // Badges read in sentence case.
        let badge = rule(".badge");
        assert!(!badge.contains("text-transform") && !badge.contains("letter-spacing"));
        assert!(script.contains("running: 'Running', blocked: 'Blocked', done: 'Shipped'"));
        // Delete on cards and rows is a text button; only the confirm dialog's Delete is red.
        assert_eq!(script.matches("class=\"toggle danger\"").count(), 1);
        assert!(!script.contains("toggle danger delete"));
        assert!(rule(".text-button").contains("text-decoration: underline"));
        // 16 px of line height plus 4 px of padding above and below: a 24 px target.
        assert!(rule(".text-button").contains("padding: var(--space-2)"));
        assert!(rule(".text-button:focus-visible").contains("outline:"));
        // The Theme switch renders in the footer, not in the toolbar.
        let footer = page
            .split("<footer>")
            .nth(1)
            .and_then(|rest| rest.split("</footer>").next())
            .expect("a footer");
        assert!(footer.contains("<div id=\"theme-switch\"></div>"));
        assert!(script.contains("document.querySelector('#theme-switch')"));
        let toolbar = script
            .split("<div class=\"toolbar\">")
            .nth(1)
            .and_then(|rest| rest.split("</div>").next())
            .expect("a toolbar");
        assert!(!toolbar.contains("ThemeSwitch"));
        // Cards drop the labels the detail sheet keeps.
        assert!(!script.contains("Activity: ${run.activity}"));
        // Stage descriptions live in Help, not on the board.
        assert!(!script.contains("group-description") && !script.contains("compact-description"));
    }

    #[test]
    fn the_address_constant_matches_the_default_port() {
        assert_eq!(ADDRESS, format!("127.0.0.1:{DEFAULT_PORT}"));
    }

    #[rocket::async_test]
    async fn a_dashboard_on_another_port_checks_host_origin_and_token_against_that_port() {
        let dir = TestDir::new();
        let api = Client::tracked(build_rocket(&dir.0.path().join("runs.json"), 4400).unwrap())
            .await
            .unwrap();
        let page = api
            .get("/")
            .header(Header::new("Host", "127.0.0.1:4400"))
            .dispatch()
            .await;
        assert_eq!(page.status(), HttpStatus::Ok);
        let html = page.into_string().await.unwrap();
        let token = html
            .split("name=\"session-token\" content=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the page carries the session token")
            .to_string();
        let delete = |host: &'static str, origin: &'static str| {
            api.delete("/api/runs/1")
                .header(Header::new("Host", host))
                .header(Header::new("Origin", origin))
                .header(Header::new(TOKEN_HEADER, token.clone()))
                .dispatch()
        };
        // Right port: passes the checks (the Run does not exist, so 404).
        assert_eq!(
            delete("127.0.0.1:4400", "http://127.0.0.1:4400")
                .await
                .status(),
            HttpStatus::NotFound
        );
        // The default port's authority is no longer trusted.
        assert_eq!(
            delete("127.0.0.1:4311", "http://127.0.0.1:4311")
                .await
                .status(),
            HttpStatus::Forbidden
        );
        assert_eq!(
            delete("127.0.0.1:4400", "http://127.0.0.1:4311")
                .await
                .status(),
            HttpStatus::Forbidden
        );
    }

    #[rocket::async_test]
    async fn the_page_opens_on_the_requested_project_expanded_and_first() {
        let script = asset("/assets/app.js").await;
        assert!(script.contains("new URLSearchParams(location.search).get('project')"));
        // Blocked Runs come first, then the requested Project's Runs, then the most recent.
        assert!(
            script.contains("blockedFirst(a, b) || first(b.run) - first(a.run) || compare(a, b)")
        );
    }

    #[rocket::async_test]
    async fn notifications_are_opt_in_skip_runs_already_waiting_and_survive_missing_permission() {
        let script = asset("/assets/app.js").await;
        // The message names the project and title.
        assert!(script.contains(
            "`Needs you in ${names.get(projectOf(run)) ?? projectOf(run)}: ${run.title}`"
        ));
        // Runs waiting on the first load are remembered, not announced: nothing is sent while `known` is unset.
        assert!(script.contains("known.current !== null && state === 'on'"));
        assert!(script.contains("if (known.current.has(runKey(run))) continue;"));
        assert!(script.contains("known.current = new Set("));
        // Opt-in: asking for permission happens only from the button, and the choice is remembered safely.
        assert_eq!(script.matches("requestPermission()").count(), 1);
        assert!(script.contains("localStorage.setItem('notifications', 'on')"));
        // Unsupported and denied permission disable the button instead of throwing.
        assert!(script.contains("typeof Notification === 'function'"));
        assert!(script.contains("'unsupported'"));
        assert!(script.contains("'denied'"));
        assert!(script.contains("disabled=${disabled}"));
        // Every browser API that can throw is guarded.
        for call in [
            "new Notification(",
            "Notification.requestPermission()",
            "localStorage.getItem('notifications')",
        ] {
            let at = script.find(call).unwrap();
            assert!(script[..at].rfind("try {").is_some(), "{call} is unguarded");
            assert!(script[at..].contains("catch"), "{call} has no catch");
        }
    }

    #[rocket::async_test]
    async fn the_detail_closes_with_escape_the_backdrop_or_a_button_and_returns_focus() {
        let script = asset("/assets/app.js").await;
        // Escape closes a native modal dialog, which fires `close`.
        assert!(script.contains("addEventListener('close'"));
        // A click on the dialog itself is a click on the backdrop.
        assert!(script.contains("event.target === event.currentTarget"));
        assert!(script.contains("class=\"toggle detail-close\""));
        // Focus goes back to the card that was opened.
        assert!(script.contains("[data-run-key="));
        assert!(
            script.contains("CSS.escape("),
            "the id is escaped in the selector"
        );
        assert!(script.contains(".focus()"));
    }

    #[rocket::async_test]
    async fn the_detail_is_a_right_hand_sheet_over_a_light_backdrop_that_stops_page_scroll() {
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        assert!(rules.contains(".detail::backdrop {"));
        assert!(rules.contains("background: var(--color-overlay)"));
        assert!(rules.contains(".detail {"));
        assert!(
            rules.contains("margin: 0 0 0 auto"),
            "the sheet sits on the right"
        );
        assert!(rules.contains("body:has(.detail[open]) { overflow: hidden; }"));
        // The whole row is the click target, with its links kept above it.
        assert!(rules.contains(".line-open::after {"));
        assert!(rules.contains(".line-row .meta a { position: relative; z-index: 1; }"));
    }

    #[rocket::async_test]
    async fn the_open_detail_follows_its_run_by_id_and_closes_when_the_run_goes() {
        let script = asset("/assets/app.js").await;
        // Looked up by id in the whole list, so refreshes update it and the filters never hide it.
        assert!(script.contains("(runs ?? []).find((run) => runKey(run) === openKey)"));
        assert!(
            !script.contains("openRun = indexed"),
            "not the filtered list"
        );
        // A Run that is no longer listed closes the detail, so it cannot reappear later.
        assert!(script.contains("if (openKey !== null && runs && !openRun) setOpenKey(null)"));
    }

    #[rocket::async_test]
    async fn the_detail_fills_the_screen_with_a_back_button_up_to_1024_px() {
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        // Up to 1024 px, the same limit as compact mode, the sheet is the whole screen.
        let compact = rules
            .split("@media (max-width: 1024px)")
            .skip(1)
            .find(|block| block.contains(".detail {"))
            .expect("a compact detail layout");
        let compact = &compact[..compact.find("\n}").unwrap()];
        assert!(compact.contains(".detail { width: 100%; }"));
        // One label is shown at a time, so only one is exposed to assistive technology.
        assert!(compact.contains(".detail-close-wide { display: none; }"));
        assert!(rules.contains(".detail-close-compact { display: none; }"));
        assert!(compact.contains(".detail-close-compact { display: inline; }"));
        let script = asset("/assets/app.js").await;
        assert!(script.contains("<span class=\"detail-close-wide\">Close</span>"));
        assert!(script.contains("<span class=\"detail-close-compact\">Back</span>"));
    }

    #[rocket::async_test]
    async fn only_a_click_that_starts_and_ends_on_the_backdrop_closes_the_detail() {
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        // The body fills the sheet, so the empty area below short content is not the backdrop.
        let body = rules
            .lines()
            .find(|line| line.starts_with(".detail-body {"))
            .expect("a detail body rule");
        assert!(body.contains("min-height: 100%"));
        assert!(body.contains("box-sizing: border-box"));
        // A drag that starts inside the sheet and ends on the backdrop is not a backdrop click.
        let script = asset("/assets/app.js").await;
        assert!(script.contains("onPointerDown="));
        assert!(script.contains("pressedBackdrop.current"));
        // The handlers the dialog keeps never go stale.
        assert!(script.contains("closing.current = onClose"));
    }

    #[rocket::async_test]
    async fn focus_has_somewhere_to_go_when_the_opened_card_cannot_take_it() {
        let script = asset("/assets/app.js").await;
        assert!(script.contains("document.querySelector('main')?.focus()"));
        assert!(script.contains("<main tabindex=\"-1\">"));
        let css = stylesheet().await;
        assert!(css.contains("main:focus { outline: none; }"));
    }

    #[rocket::async_test]
    async fn the_detail_fields_stack_on_very_narrow_screens() {
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        let narrow = rules
            .split("@media (max-width: 30rem)")
            .nth(1)
            .expect("a very narrow layout");
        assert!(narrow.contains(".reports li { grid-template-columns: minmax(0, 1fr);"));
    }

    #[rocket::async_test]
    async fn dashboard_serves_barlow_itself_and_uses_its_own_palette() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        // Both weights come from the dashboard itself, so the page works offline.
        for weight in ["400", "600"] {
            let response = api
                .get(format!("/assets/vendor/barlow-{weight}.woff2"))
                .header(Header::new("Host", "127.0.0.1:4311"))
                .dispatch()
                .await;
            assert_eq!(response.status(), HttpStatus::Ok, "barlow {weight}");
            assert_eq!(response.content_type(), Some(ContentType::WOFF2));
            let bytes = response.into_bytes().await.unwrap();
            assert!(bytes.starts_with(b"wOF2"), "barlow {weight} is woff2");
        }
        let css = stylesheet().await;
        for weight in ["400", "600"] {
            assert!(css.contains(&format!(
                "src: url(/assets/vendor/barlow-{weight}.woff2) format('woff2');"
            )));
            assert!(css.contains(&format!("font-weight: {weight};")));
        }
        assert!(css.contains("font-display: swap;"));
        assert!(
            !css.contains("url(http"),
            "no font or image from the network"
        );
        assert!(css.contains("--font-sans: 'Barlow', system-ui, sans-serif;"));
        // The six named colours, light values as in the design proposal.
        let (layer, rules) = split_tokens(&css);
        let light = layer
            .split(":root {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .unwrap();
        for (name, value) in [
            ("floor", "#E8EBEE"),
            ("panel", "#FFFFFF"),
            ("slate", "#1B2633"),
            ("blue", "#0A6CE0"),
            ("amber", "#F2B705"),
            ("red", "#C4314B"),
        ] {
            assert!(
                light.contains(&format!("--color-{name}: {value};")),
                "--color-{name}"
            );
        }
        assert!(
            !layer.contains("--colorNeutral"),
            "the Fluent layer is gone"
        );
        // Needs you is amber with dark text; red is left for errors and the confirm Delete.
        let rule = |selector: &str| {
            rules
                .lines()
                .find(|line| line.starts_with(&format!("{selector} {{")))
                .unwrap_or_else(|| panic!("no {selector} rule"))
                .to_string()
        };
        for selector in [".badge.blocked", ".andon.lit"] {
            let needs_you = rule(selector);
            assert!(needs_you.contains("var(--color-amber)"), "{selector}");
            assert!(
                needs_you.contains("color: var(--color-on-amber)"),
                "{selector}"
            );
        }
        assert!(rule(".toggle.danger").contains("var(--color-red)"));
        assert!(rule("#message.error").contains("var(--color-red)"));
        // The line sits flat on the floor; only dialogs and menus cast a shadow.
        assert!(!rule(".line").contains("box-shadow"));
        assert!(
            !rules.contains("box-shadow: inset"),
            "a blocked row is marked with a bar, not a shadow"
        );
        assert_eq!(
            rules.matches("var(--shadow-dialog)").count(),
            4,
            "confirm, detail, Reader and the Projects menu"
        );
        let license = include_str!("../third-party/barlow-LICENSE");
        assert!(license.contains("SIL Open Font License"));
    }

    #[rocket::async_test]
    async fn the_andon_strip_says_whether_anything_needs_you() {
        let script = asset("/assets/app.js").await;
        let andon = script
            .split("function Andon(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("an Andon component");
        // Quiet: one line that says so, with how many Runs are on the line. Lit: how many need you.
        assert!(andon.contains("'Nothing needs you.'"));
        assert!(andon.contains("on the line.`"));
        assert!(andon.contains("'1 run needs you'"));
        assert!(andon.contains("`${waiting.length} runs need you`"));
        // Only whether and how many Runs need you is announced: not the list, and not the count on the line,
        // so a Run starting or finishing elsewhere is not read out.
        assert!(andon.contains("<span role=\"status\">${needs}</span>"));
        assert_eq!(andon.matches("role=\"status\"").count(), 1);
        // Each waiting Run shows its title, Project and status reason, and opens its detail.
        assert!(andon.contains("class=\"andon-run\""));
        assert!(andon.contains("onClick=${() => onOpen(runKey(run))}"));
        assert!(andon.contains("<${ProjectName} path=${projectOf(run)} />"));
        assert!(andon.contains("run.blocking_reason"));
        // The Needs you filter lives in the strip, and stays reachable while it is on.
        assert!(andon.contains("class=\"toggle andon-filter\""));
        assert!(andon.contains("aria-pressed=${String(filtered)}"));
        assert!(andon.contains("waiting.length || filtered"));
        assert!(
            !script.contains("NeedsYouToggle"),
            "the toolbar toggle is gone"
        );
        // Hidden Projects stay out of the strip.
        assert!(script.contains("<${Andon}\n            waiting=${visibleRuns"));
        // Focus goes back to the control that opened the detail, wherever it was; failing that, to the Run's
        // row on the board before its entry in the strip.
        assert!(script.contains("opener.current = document.activeElement"));
        assert!(script.contains(
            "document.querySelector(`.line [data-run-key=\"${key}\"], .shipped [data-run-key=\"${key}\"]`)"
        ));
        let css = stylesheet().await;
        let lit = css
            .lines()
            .find(|line| line.starts_with(".andon.lit {"))
            .expect("a lit andon rule");
        assert!(lit.contains("background: var(--color-amber)"));
        assert!(lit.contains("color: var(--color-on-amber)"));
    }

    #[rocket::async_test]
    async fn the_line_shows_each_active_run_on_a_four_station_rail() {
        let script = asset("/assets/app.js").await;
        assert!(script.contains("const STATIONS = ['define', 'build', 'check', 'finish'];"));
        let row = script
            .split("function LineRow(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a LineRow component");
        // The title opens the detail and is found again by the Run key.
        assert!(row.contains("class=\"line-open\" data-run-key=${runKey(run)}"));
        // The rail is drawn for sighted users; its position is also given in words.
        assert!(row.contains("<div class=\"rail\" aria-hidden=\"true\">"));
        assert!(row.contains("<p class=\"where\">${where}</p>"));
        assert!(row.contains("`${station}, blocked`"));
        // Project, ticket progress, Rework, pull request and age sit in one meta line.
        for part in [
            "<${ProjectName} path=${projectOf(run)} />",
            "<${ContextChips} run=${run} />",
            "reworkLabel(",
            "<${ContextLinks} run=${run} />",
            "<${Updated} run=${run} now=${now} />",
        ] {
            assert!(row.contains(part), "row misses {part}");
        }
        // Active Runs only, blocked first, and the requested Project before the rest.
        assert!(script.contains("run.status !== 'done'"));
        assert!(script.contains("const first = (run) => Number(projectOf(run) === wanted);"));
        assert!(script.contains(
            "const blockedFirst = (a, b) => Number(needsYou(b.run)) - Number(needsYou(a.run));"
        ));
        // The old grid and its Stage buttons are gone.
        for gone in [
            "board-headers",
            "StageButtons",
            "project-row",
            "collapsed-projects",
        ] {
            assert!(!script.contains(gone), "{gone} is still there");
        }
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        assert!(!rules.contains("repeat(5,"), "no five-column board");
        assert!(rules.contains(".rail { "));
        // The current station is a marker placed by the Run's station number.
        assert!(rules.contains("calc((var(--station) + 0.5) * 25%)"));
        assert!(rules.contains(".line-row.blocked .rail-marker {"));
    }

    #[rocket::async_test]
    async fn the_projects_menu_hides_restores_and_deletes_projects() {
        let script = asset("/assets/app.js").await;
        let menu = script
            .split("function ProjectsMenu(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a ProjectsMenu component");
        assert!(
            menu.contains("<details class=\"projects-menu\" ref=${menu} onKeyDown=${onKeyDown}>")
        );
        assert!(menu.contains("<summary>Projects (${projects.length})</summary>"));
        assert!(menu.contains("const name = names.get(project) ?? project;"));
        assert!(menu.contains("aria-label=${`${hidden ? 'Restore' : 'Hide'} ${name}`}"));
        assert!(menu.contains("aria-label=${`Delete ${name}`}"));
        assert!(menu.contains("onClick=${() => onDelete(project)}"));
        // Hiding is remembered in this browser, and deleting a Project still asks first.
        assert!(script.contains("localStorage.setItem('hidden-projects'"));
        assert!(script.contains("onDelete=${(project) => setPending({ project, runs: runs.filter((run) => projectOf(run) === project) })}"));
        assert!(script.contains("<${ProjectsMenu}"));
        // It closes with Escape (focus back on its summary) and with a click elsewhere.
        assert!(menu.contains("event.key === 'Escape'"));
        assert!(menu.contains("document.addEventListener('pointerdown', onOutside)"));
        let css = stylesheet().await;
        assert!(css.contains(".toolbar { position: relative;"));
        assert!(css.contains(".projects-menu { position: static; }"));
    }

    #[rocket::async_test]
    async fn shipped_runs_are_listed_by_day_and_cleared_after_confirming() {
        let script = asset("/assets/app.js").await;
        let log = script
            .split("function ShippedLog(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a ShippedLog component");
        assert!(log.contains("<h2 id=\"shipped-title\">Shipped"));
        assert!(log.contains("<table class=\"shipped-table\">"));
        assert!(log.contains("<caption class=\"visually-hidden\">"));
        for column in ["Run", "Project", "Pull request", "Fresh tokens", "Shipped"] {
            assert!(
                log.contains(&format!("<th scope=\"col\">{column}</th>")),
                "{column}"
            );
        }
        // One group per day, newest first; a Run without a time goes last.
        assert!(log.contains("<th scope=\"rowgroup\" colspan=\"5\">${group.label}</th>"));
        let day = script
            .split("function dayLabel(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a dayLabel function");
        for label in ["'Today'", "'Yesterday'", "'Undated'"] {
            assert!(day.contains(label), "{label}");
        }
        // Unknown usage is never shown as zero.
        assert!(log.contains("tokens === null ? 'n/a' : compactTokens(tokens)"));
        // Clear shipped asks first, naming how many.
        assert!(log.contains("Clear shipped…"));
        assert!(script.contains(
            "`Clear ${pending.runs.length} shipped ${pending.runs.length === 1 ? 'run' : 'runs'}?`"
        ));
        // Single Runs are deleted from their detail, through the same dialog.
        let detail = script
            .split("function Detail(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .unwrap();
        assert!(detail.contains("onClick=${() => onDelete(run)}"));
        assert!(script.contains("onDelete=${(run) => setPending({ runs: [run] })}"));
        assert!(!script.contains("DoneCard"), "rows have no Delete");
        // People read Shipped; the status stays done.
        assert!(script.contains("done: 'Shipped'"));
        assert!(script.contains("run.status === 'done'"));
        let help = asset("/assets/help.js").await;
        assert!(!help.contains("Done column"));
        assert!(help.contains("['Shipped', "));
        let glossary = include_str!("../GLOSSARY.md");
        for term in ["Shipped", "Andon strip", "The line"] {
            assert!(glossary.contains(&format!("**{term}**")), "{term}");
        }
    }

    #[rocket::async_test]
    async fn the_rail_marker_slides_to_a_new_station_unless_motion_is_reduced() {
        let css = stylesheet().await;
        let (layer, rules) = split_tokens(&css);
        assert!(layer.contains("--duration-move: 200ms;"));
        let marker = rules
            .lines()
            .find(|line| line.starts_with(".rail-marker {"))
            .expect("a rail marker rule");
        // The marker is placed by --station, so a new Stage moves it; a new row has nothing to move from.
        assert!(marker.contains("left: calc((var(--station) + 0.5) * 25%)"));
        assert!(marker.contains("transition: left var(--duration-move) var(--ease)"));
        let reduced = rules
            .split("@media (prefers-reduced-motion: reduce)")
            .last()
            .expect("a reduced-motion block");
        assert!(
            reduced.contains(".rail-marker"),
            "no slide under reduced motion"
        );
        // Rows are keyed by Run, so a refresh keeps the element and only its station changes.
        let script = asset("/assets/app.js").await;
        assert!(script.contains("<${LineRow} key=${runKey(entry.run)}"));
        assert!(script.contains("style=${`--station: ${index}`}"));
    }

    /// The relative luminance of a `#RRGGBB` colour, as WCAG 2 defines it.
    fn luminance(hex: &str) -> f64 {
        let channel = |at: usize| {
            let value = f64::from(u8::from_str_radix(&hex[at..at + 2], 16).unwrap()) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn contrast_follows_the_wcag_formula() {
        // Worked examples: black on white is 21:1, and #767676 on white is the classic 4.54:1.
        assert!((contrast("#000000", "#FFFFFF") - 21.0).abs() < 0.01);
        assert!((contrast("#767676", "#FFFFFF") - 4.54).abs() < 0.01);
    }

    #[rocket::async_test]
    async fn text_colours_have_enough_contrast_in_both_themes() {
        let css = stylesheet().await;
        let (layer, _) = split_tokens(&css);
        let colour_in = |marker: &str, name: &str| -> String {
            let start = layer.find(marker).unwrap();
            let body = &layer[start..start + layer[start..].find('}').unwrap()];
            let at = body
                .find(&format!("--color-{name}: "))
                .unwrap_or_else(|| panic!("no --color-{name} in {marker}"));
            body[at..].split(": ").nth(1).unwrap()[..7].to_string()
        };
        for marker in [":root {", ":root[data-theme=\"dark\"] {"] {
            let colour = |name: &str| colour_in(marker, name);
            for (text, background, minimum) in [
                ("slate", "panel", 4.5),
                ("muted", "panel", 4.5),
                ("muted", "floor", 4.5),
                ("muted", "tint", 4.5),
                ("blue", "panel", 4.5),
                ("red", "floor", 4.5),
                ("on-amber", "amber", 4.5),
                ("on-blue", "blue", 4.5),
                ("on-red", "red", 4.5),
                ("slate", "tint", 4.5),
            ] {
                let ratio = contrast(&colour(text), &colour(background));
                assert!(
                    ratio >= minimum,
                    "{text} on {background} in {marker}: {ratio:.2}"
                );
            }
        }
    }

    #[rocket::async_test]
    async fn the_token_table_gets_the_full_width_of_the_detail() {
        // The breakdown has ten columns, so it sits in a disclosure that spans the sheet, open while the Run is active.
        let script = asset("/assets/app.js").await;
        assert!(script.contains("<details class=\"usage\" open=${active}>"));
        let css = stylesheet().await;
        assert!(css.contains(".usage { margin:"));
        // The table fills its row and its numbers line up.
        let table = css
            .lines()
            .find(|line| line.starts_with(".usage-table {"))
            .expect("a usage table rule");
        assert!(table.contains("width: 100%"));
        assert!(table.contains("font-variant-numeric: tabular-nums"));
    }

    #[rocket::async_test]
    async fn the_token_table_shows_tickets_as_short_links_so_it_fits() {
        // A ticket URL in the breakdown made the table wider than the sheet. It shows as #202, with the URL as a tooltip.
        let script = asset("/assets/app.js").await;
        let short = script
            .split("function shortRef(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a shortRef function");
        // GitHub issues and pull requests, and Azure Boards work items, by number; anything else by its last part.
        assert!(short.contains("/(?:issues|pull|pullrequest|workitems\\/edit)\\/(\\d+)\\/?$/"));
        assert!(short.contains("`#${number[1]}`"));
        assert!(script.contains("<td><${TicketRef} value=${rowBucket(row)} /></td>"));
        assert!(script.contains("title=${url.href}"));
        // Model and ticket may wrap, so a long name never widens the table.
        let css = stylesheet().await;
        assert!(css.contains(
            ".usage-table td:nth-child(2), .usage-table td:nth-child(3) { white-space: normal; overflow-wrap: anywhere; }"
        ));
    }

    #[rocket::async_test]
    async fn links_share_one_http_check_and_short_ticket_links_say_what_they_are() {
        let script = asset("/assets/app.js").await;
        // One place decides what becomes a link: an http(s) URL, nothing else.
        assert!(script.contains("function httpUrl(value) {"));
        assert_eq!(
            script
                .matches("url.protocol === 'http:' || url.protocol === 'https:'")
                .count(),
            1,
            "the protocol check lives only in httpUrl"
        );
        for component in ["function RefLink(", "function TicketRef("] {
            let body = script
                .split(component)
                .nth(1)
                .and_then(|rest| rest.split("\n}\n").next())
                .unwrap_or_else(|| panic!("no {component}"));
            assert!(body.contains("httpUrl("), "{component} uses httpUrl");
            assert!(
                !body.contains("new URL("),
                "{component} parses no URL itself"
            );
        }
        // A screen reader hears "Ticket #202", not just "#202".
        assert!(script.contains("<span class=\"visually-hidden\">Ticket </span>${shortRef(url)}"));
    }

    #[rocket::async_test]
    async fn the_run_list_carries_the_pull_request_state_the_server_stored() {
        use crate::pr_state::{PrState, PrStatus};
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let store = RunStore::open(&path).unwrap();
        let run = store
            .create(NewRun {
                title: "t".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        let pr = "https://github.com/o/r/pull/7";
        store
            .update(
                &run.id,
                serde_json::from_str(&format!(
                    r#"{{"stage":"finish","status":"done","pr":"{pr}","summaries":{{"finish":"Shipped"}}}}"#
                ))
                .unwrap(),
            )
            .unwrap();
        store
            .set_pr_status(
                &run.id,
                PrStatus {
                    url: pr.into(),
                    state: PrState::Merged,
                    reason: None,
                    checked_at: 42,
                },
            )
            .unwrap();
        drop(store);
        let api = Client::tracked(build_rocket(&path, DEFAULT_PORT).unwrap())
            .await
            .unwrap();
        let list: serde_json::Value = api
            .get("/api/runs")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_json()
            .await
            .unwrap();
        assert_eq!(
            list[0]["pr_status"],
            serde_json::json!({"url": pr, "state": "merged", "checked_at": 42})
        );
        // An update through the API cannot set it.
        let response = api
            .post(format!("/api/runs/{}", run.id))
            .header(cli_token_header(&api))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(r#"{"stage":"finish","status":"done","summaries":{"finish":"Shipped"},"pr_status":{"state":"open"}}"#)
            .dispatch()
            .await;
        assert_ne!(response.status(), HttpStatus::Ok);
    }

    /// Answers every `gh` call with a merged pull request.
    struct MergedGh;

    impl crate::pr_wait::Gh for MergedGh {
        fn run(&self, _args: &[&str]) -> Result<crate::pr_wait::GhOutput, crate::pr_wait::GhError> {
            Ok(crate::pr_wait::GhOutput {
                code: Some(0),
                stdout: r#"{"state":"MERGED","isDraft":false}"#.into(),
                stderr: String::new(),
            })
        }
    }

    #[rocket::async_test]
    async fn the_dashboard_server_reads_pull_request_states_in_the_background() {
        use crate::pr_state::PrState;
        let dir = TestDir::new();
        let store = RunStore::open(&dir.0.path().join("runs.json")).unwrap();
        let run = store
            .create(NewRun {
                title: "t".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        store
            .update(
                &run.id,
                serde_json::from_str(
                    r#"{"stage":"finish","status":"done","summaries":{"finish":"Shipped"},"pr":"https://github.com/o/r/pull/7"}"#,
                )
                .unwrap(),
            )
            .unwrap();
        let watcher = watch_pr_states(
            store.clone(),
            std::sync::Arc::new(MergedGh),
            std::time::Duration::from_millis(10),
        );
        // Wait for the first pass, with a deadline instead of a fixed sleep.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let state = loop {
            let stored = store.list().unwrap()[0].pr_status.clone();
            if let Some(status) = stored {
                break status.state;
            }
            assert!(std::time::Instant::now() < deadline, "no state was stored");
            rocket::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        watcher.abort();
        assert_eq!(state, PrState::Merged);
    }

    #[rocket::async_test]
    async fn the_dashboard_shows_each_pull_requests_state_in_words() {
        let script = asset("/assets/app.js").await;
        assert!(script.contains(
            "const PR_STATE = { open: 'Open', draft: 'Draft', merged: 'Merged', closed: 'Closed' };"
        ));
        let label = script
            .split("function PrStateLabel(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a PrStateLabel component");
        // Only a state read for the pull request the Run records now, and never an unknown one, gets a label.
        assert!(label.contains("status.url !== run.pr"));
        assert!(label.contains("!PR_STATE[status.state]"));
        assert!(label.contains("<span class=${`pr-state ${status.state}`}>"));
        // Beside the link on the line and in the Shipped log, and in words next to the link in the detail.
        assert!(script.contains("<${ContextLinks} run=${run} /><${PrStateLabel} run=${run} />"));
        assert!(script.contains("<${PrStateLabel} run=${run} /></td>"));
        let detail = script
            .split("function PrLink(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a PrLink component");
        assert!(detail.contains("status.reason"));
        assert!(detail.contains("PR_STATE[status.state]"));
        let css = stylesheet().await;
        for state in ["open", "draft", "merged", "closed"] {
            assert!(css.contains(&format!(".pr-state.{state} {{")), "{state}");
        }
    }

    #[rocket::async_test]
    async fn the_detail_shows_the_status_reason_only_while_blocked_and_the_activity_only_while_active(
    ) {
        // The workflows clear the reason when work resumes; a Run that is not blocked shows none, and a shipped Run no activity.
        let script = asset("/assets/app.js").await;
        assert!(script.contains(
            "${blocked && run.blocking_reason ? html`<p class=\"status-reason\">${run.blocking_reason}</p>` : null}"
        ));
        assert!(script.contains("${active && run.activity ?"));
    }

    #[rocket::async_test]
    async fn the_toolbar_andon_strip_and_line_stay_in_view_without_taking_the_screen() {
        let script = asset("/assets/app.js").await;
        // One sticky block inside the main landmark: the toolbar, the andon strip, the message and the line.
        let top = script
            .split("<div class=\"top\" ref=${top}>")
            .nth(1)
            .expect("a sticky top block")
            .split("${runs && !runs.length ? html`<${Welcome}")
            .next()
            .unwrap();
        for part in [
            "<div class=\"toolbar\">",
            "<${Andon}",
            "<p id=\"message\"",
            "<section class=\"line\"",
        ] {
            assert!(top.contains(part), "the top holds {part}");
        }
        assert!(
            !top.contains("<${ShippedLog}"),
            "the Shipped log scrolls under it"
        );
        assert!(
            script.find("<main tabindex=\"-1\">").unwrap()
                < script.find("<div class=\"top\"").unwrap()
        );
        // Its height is published, so focus below it is scrolled into view under it, not behind it.
        assert!(script.contains("new ResizeObserver("));
        assert!(script.contains("setProperty('--top-height', `${element.offsetHeight}px`)"));
        assert!(script.contains("setProperty('--toolbar-height', `${toolbar.offsetHeight}px`)"));
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        let rule = |selector: &str| {
            rules
                .lines()
                .find(|line| line.starts_with(&format!("{selector} {{")))
                .unwrap_or_else(|| panic!("no {selector} rule"))
                .to_string()
        };
        let sticky = rule(".top");
        assert!(sticky.contains("position: sticky; top: 0;"));
        assert!(
            sticky.contains("background: var(--color-floor)"),
            "content does not show through"
        );
        // Never more than 60% of the window: the rows and the waiting Runs scroll inside instead.
        assert!(sticky.contains("max-height: 60dvh"));
        assert!(
            sticky.contains("box-sizing: border-box"),
            "the cap includes the padding"
        );
        assert!(rule(".line-rows").contains("overflow-y: auto"));
        assert!(rule(".andon-list").contains("overflow-y: auto"));
        // At the cap the line gives up room first, down to about one row; then the waiting Runs.
        let line = rule(".top > .line");
        assert!(
            line.contains("flex: 0 1000 auto") && line.contains("min-height: min(8rem, 20dvh)")
        );
        // Browsers without dvh fall back to vh rather than dropping the minimum.
        assert!(line.contains("min-height: min(8rem, 20vh); min-height: min(8rem, 20dvh)"));
        let andon = rule(".top > .andon");
        assert!(andon.contains("flex: 0 1 auto") && andon.contains("min-height: 0"));
        // Above the footer, so the Projects menu panel is never covered.
        assert!(sticky.contains("z-index: 3"));
        // Focus rings inside the scrolling lists sit inside their control, where the list cannot clip them.
        assert!(rule(".andon-run:focus-visible")
            .contains("outline-offset: calc(-2 * var(--stroke-thick))"));
        assert!(rule(".line-open:focus-visible::after")
            .contains("outline-offset: calc(-1 * var(--stroke-thick))"));
        assert!(rule(".line-row .meta a:focus-visible").contains("outline-offset: 0"));
        // Narrow screens: the toolbar scrolls away, the andon strip and the line stay.
        let narrow = rules
            .split("@media (max-width: 48rem) {\n")
            .skip(1)
            .find(|block| {
                block
                    .split("\n}")
                    .next()
                    .is_some_and(|body| body.contains(".top {"))
            })
            .and_then(|block| block.split("\n}").next())
            .expect("a narrow-screen sticky rule");
        // The toolbar and its bottom margin scroll away, so the strip sits just below the window's top.
        assert!(narrow.contains("top: calc(-1 * (var(--toolbar-height) + var(--space-6)))"));
        assert!(
            narrow.contains("scroll-padding-top: calc(var(--top-height) - var(--toolbar-height))")
        );
        assert!(
            rule("html").contains("scroll-padding-top: calc(var(--top-height) + var(--space-4))")
        );
        // In a very short window, such as at 400% zoom, it scrolls with the page.
        let short = rules
            .split("@media (max-height: 30rem) {\n")
            .nth(1)
            .and_then(|rest| rest.split("\n}").next())
            .expect("a short-window block");
        assert!(short.contains(".top { position: static; max-height: none; }"));
        assert!(short.contains("html { scroll-padding-top: 0; }"));
        // On a phone the Shipped table keeps readable columns and scrolls sideways in its own region.
        assert!(rule(".shipped-table").contains("min-width: 36rem"));
        // The page column may be narrower than that table, so only the table's region scrolls, not the page.
        assert!(rule("#app").contains("min-width: 0"));
        // A closed Projects menu does not lay out its panel, which would otherwise widen the page on a phone.
        assert!(rules.contains(".projects-menu:not([open]) .projects-panel { display: none; }"));
        // Visually hidden labels in a scrolled table are positioned inside it, so they cannot widen the page.
        assert!(rule(".table-scroll").contains("position: relative"));
    }

    #[rocket::async_test]
    async fn the_page_reloads_itself_when_a_new_server_answers() {
        // The page knows which server served it, by process id, the same id /api/info reports.
        let page = dashboard_page().await;
        assert!(!page.contains(PID_PLACEHOLDER), "placeholder left in page");
        assert!(page.contains(&format!(
            "<meta name=\"server-pid\" content=\"{}\">",
            std::process::id()
        )));
        let script = asset("/assets/app.js").await;
        let hook = script
            .split("function useReloadOnNewServer(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("a reload hook");
        assert!(hook.contains("fetch('/api/info', { cache: 'no-store' })"));
        assert!(hook.contains("changed = String(info.pid) !== page;"));
        // Never while a dialog or the Projects menu is open, and never because the server is unreachable.
        assert!(hook.contains("if (changed && !somethingOpen()) location.reload();"));
        assert!(
            script.contains("document.querySelector('dialog[open], details.projects-menu[open]')")
        );
        assert!(hook.contains("if (!response.ok) return;"));
        assert!(script.contains("  useReloadOnNewServer();\n"));
    }

    #[rocket::async_test]
    async fn dashboard_tokens_are_all_used_and_all_declared() {
        let css = stylesheet().await;
        let (layer, rules) = split_tokens(&css);
        let declared = tokens_after(layer, ":root {");
        let used = variable_uses(rules);
        assert_eq!(
            declared.difference(&used).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "tokens no rule uses"
        );
        assert_eq!(
            used.difference(&declared).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "variables no token declares"
        );
    }

    #[rocket::async_test]
    async fn the_page_has_a_favicon_and_a_decorative_logo_with_a_readable_heading() {
        let page = dashboard_page().await;
        assert!(page
            .contains("<link rel=\"icon\" href=\"/assets/favicon.svg\" type=\"image/svg+xml\">"));
        // The logo is drawn inline, hidden from assistive technology, and the heading keeps its text.
        let heading = page
            .split("<h1>")
            .nth(1)
            .and_then(|rest| rest.split("</h1>").next())
            .expect("a heading");
        assert!(heading.contains("<svg class=\"logo\""));
        assert!(
            heading.contains("aria-hidden=\"true\"") && heading.contains("focusable=\"false\"")
        );
        assert!(heading.contains("<span class=\"visually-hidden\">TDS Factory</span>"));
        // Its gradient has an id of its own, and only the dark parts follow the theme.
        assert!(
            heading.contains("id=\"tds-logo-blue\"") && heading.contains("url(#tds-logo-blue)")
        );
        let css = stylesheet().await;
        assert!(css.contains(".logo .ink { fill: var(--color-slate); }"));
    }

    #[rocket::async_test]
    async fn the_footer_with_the_version_stays_in_view() {
        let css = stylesheet().await;
        let rule = |selector: &str| {
            css.lines()
                .find(|line| line.starts_with(&format!("{selector} {{")))
                .unwrap_or_else(|| panic!("no {selector} rule"))
                .to_string()
        };
        // Stuck to the bottom of the window while the page scrolls, with a background so content does not show through.
        let footer = rule("footer");
        assert!(footer.contains("position: sticky") && footer.contains("bottom: 0"));
        // Above card buttons and links (z-index 1), so nothing paints over it.
        assert!(footer.contains("z-index: 2"));
        assert!(footer.contains("background:"));
        // At the bottom of a short page: the body is a column at least as tall as the window.
        let body = rule("body");
        assert!(body.contains("min-height: 100dvh") && body.contains("flex-direction: column"));
        assert!(footer.contains("margin-top: auto"));
        // A focused element is not left behind the footer.
        assert!(rule("html").contains("scroll-padding-bottom"));
    }

    #[rocket::async_test]
    async fn dashboard_theme_toggle_is_a_labelled_radio_group_remembered_safely() {
        let page = dashboard_page().await;
        assert!(page.contains("<legend>Theme</legend>"));
        assert!(page.contains("type=\"radio\""));
        assert!(page.contains("name=\"theme\""));
        for value in ["system", "light", "dark"] {
            assert!(
                page.contains(&format!("['{value}', ")),
                "missing {value} option"
            );
        }
        assert!(page.contains("return 'system';"), "System is the default");
        // The stored choice is applied before the stylesheet, so there is no flash.
        let early = page.find("localStorage.getItem('theme')").unwrap();
        assert!(early < page.find("rel=\"stylesheet\"").unwrap());
        // Every storage access is guarded so the page works without it.
        for access in ["getItem", "setItem", "removeItem"] {
            let at = page.find(access).unwrap();
            assert!(page[..at].rfind("try {").is_some(), "{access} is unguarded");
            assert!(page[at..].contains("catch"), "{access} has no catch");
        }
    }

    #[rocket::async_test]
    async fn create_update_and_restart_preserve_cli_json_contract() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let response = api
            .post("/api/runs")
            .header(cli_token_header(&api))
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
            .header(cli_token_header(&api))
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
            .header(cli_token_header(&api))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .header(Header::new("Origin", "https://example.test"))
            .body(r#"{"title":"x","project":"/"}"#)
            .dispatch()
            .await;
        assert_eq!(origin.status(), HttpStatus::Forbidden);
        let content_type = api
            .post("/api/runs")
            .header(cli_token_header(&api))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::Plain)
            .body(r#"{"title":"x","project":"/"}"#)
            .dispatch()
            .await;
        assert_eq!(content_type.status(), HttpStatus::Forbidden);
        let invalid = api
            .post("/api/runs")
            .header(cli_token_header(&api))
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

    fn cli_token_header(api: &Client) -> Header<'static> {
        let token = api.rocket().state::<CliToken>().unwrap();
        Header::new(CLI_TOKEN_HEADER, token.0.clone())
    }

    async fn session_token(api: &Client) -> String {
        let page = api
            .get("/")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap();
        assert!(!page.contains(TOKEN_PLACEHOLDER));
        page.split("name=\"session-token\" content=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the page carries the session token")
            .to_owned()
    }

    async fn delete<'a>(
        api: &'a Client,
        id: &str,
        headers: &[(&'static str, String)],
    ) -> rocket::local::asynchronous::LocalResponse<'a> {
        let mut request = api.delete(format!("/api/runs/{id}"));
        for (name, value) in headers {
            request = request.header(Header::new(*name, value.clone()));
        }
        request.dispatch().await
    }

    #[rocket::async_test]
    async fn delete_removes_only_the_record_and_survives_restart() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let store = api.rocket().state::<RunStore>().unwrap();
        let new = |title: &str| NewRun {
            title: title.into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        };
        let kept = store.create(new("kept")).unwrap();
        let token = session_token(&api).await;
        // A request without an Origin (curl) and the dashboard's own same-origin requests are allowed.
        for origin in [
            None,
            Some("http://127.0.0.1:4311"),
            Some("http://localhost:4311"),
        ] {
            let run = store.create(new("gone")).unwrap();
            let mut headers = vec![
                ("Host", "127.0.0.1:4311".to_string()),
                (TOKEN_HEADER, token.clone()),
            ];
            headers.extend(origin.map(|origin| ("Origin", origin.to_string())));
            let response = delete(&api, &run.id, &headers).await;
            assert_eq!(
                response.status(),
                HttpStatus::NoContent,
                "origin {origin:?}"
            );
        }
        let remaining = load_runs(&dir.0.path().join("runs.json")).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, kept.id);
        let restarted = client(&dir).await;
        assert_eq!(
            restarted
                .rocket()
                .state::<RunStore>()
                .unwrap()
                .list()
                .unwrap()
                .len(),
            1
        );
    }

    #[rocket::async_test]
    async fn delete_removes_a_run_in_any_stage_and_status() {
        use crate::run::{RunStatus, Stage, UpdateRun};
        let dir = TestDir::new();
        let api = client(&dir).await;
        let store = api.rocket().state::<RunStore>().unwrap();
        let token = session_token(&api).await;
        let headers = vec![
            ("Host", "127.0.0.1:4311".to_string()),
            (TOKEN_HEADER, token),
        ];
        for stage in [Stage::Define, Stage::Build, Stage::Check, Stage::Finish] {
            for status in [RunStatus::Running, RunStatus::Blocked, RunStatus::Done] {
                let run = store
                    .create(NewRun {
                        title: "stuck".into(),
                        project: "/work".into(),
                        repository: None,
                        session: None,
                        model_routing: None,
                    })
                    .unwrap();
                let update: UpdateRun =
                    serde_json::from_value(serde_json::json!({ "stage": stage, "status": status, "summaries": { "finish": "Shipped" } }))
                        .unwrap();
                store.update(&run.id, update).unwrap();
                let response = delete(&api, &run.id, &headers).await;
                assert_eq!(response.status(), HttpStatus::NoContent);
            }
        }
        assert!(store.list().unwrap().is_empty());
    }

    #[rocket::async_test]
    async fn delete_rejects_bad_host_origin_and_token_without_mutation() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let run = api
            .rocket()
            .state::<RunStore>()
            .unwrap()
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        let token = session_token(&api).await;
        let host = ("Host", "127.0.0.1:4311".to_string());
        let good = ("X-Session-Token", token.clone());
        let cases: Vec<(&str, Vec<(&'static str, String)>)> = vec![
            (
                "foreign host",
                vec![("Host", "attacker.example".into()), good.clone()],
            ),
            (
                "wrong port",
                vec![("Host", "127.0.0.1:4312".into()), good.clone()],
            ),
            ("no host", vec![good.clone()]),
            (
                "foreign origin",
                vec![
                    host.clone(),
                    good.clone(),
                    ("Origin", "https://example.test".into()),
                ],
            ),
            (
                "foreign origin on a local port",
                vec![
                    host.clone(),
                    good.clone(),
                    ("Origin", "http://127.0.0.1:9999".into()),
                ],
            ),
            (
                "https origin",
                vec![
                    host.clone(),
                    good.clone(),
                    ("Origin", "https://127.0.0.1:4311".into()),
                ],
            ),
            (
                "null origin",
                vec![host.clone(), good.clone(), ("Origin", "null".into())],
            ),
            ("missing token", vec![host.clone()]),
            (
                "wrong token",
                vec![host.clone(), ("X-Session-Token", "nope".into())],
            ),
            (
                "empty token",
                vec![host.clone(), ("X-Session-Token", String::new())],
            ),
        ];
        for (name, headers) in cases {
            let response = delete(&api, &run.id, &headers).await;
            assert_eq!(response.status(), HttpStatus::Forbidden, "{name}");
            assert_eq!(
                api.rocket()
                    .state::<RunStore>()
                    .unwrap()
                    .list()
                    .unwrap()
                    .len(),
                1,
                "{name} deleted the Run"
            );
        }
        let headers = [host.clone(), good.clone()];
        assert_eq!(
            delete(&api, "nope", &headers).await.status(),
            HttpStatus::BadRequest
        );
        assert_eq!(
            delete(&api, "123", &headers).await.status(),
            HttpStatus::NotFound
        );
        assert_eq!(load_runs(&dir.0.path().join("runs.json")).unwrap().len(), 1);
    }

    #[rocket::async_test]
    async fn each_server_start_has_its_own_session_token() {
        let (first, second) = (TestDir::new(), TestDir::new());
        let (first, second) = (client(&first).await, client(&second).await);
        let (a, b) = (session_token(&first).await, session_token(&second).await);
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(a, session_token(&first).await);
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
            .header(cli_token_header(&api))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(big)
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::PayloadTooLarge);
        let response = api
            .post("/api/runs/nope")
            .header(cli_token_header(&api))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(r#"{"stage":"check","status":"done"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), HttpStatus::BadRequest);
        assert_eq!(
            api.post("/api/runs/123")
                .header(cli_token_header(&api))
                .header(Header::new("Host", "127.0.0.1:4311"))
                .header(ContentType::JSON)
                .body(r#"{"stage":"check","status":"done"}"#)
                .dispatch()
                .await
                .status(),
            HttpStatus::NotFound
        );
    }

    async fn report<'a>(
        api: &'a Client,
        id: &str,
        body: &str,
    ) -> rocket::local::asynchronous::LocalResponse<'a> {
        api.post(format!("/api/runs/{id}"))
            .header(cli_token_header(api))
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
        assert!(original.rework.is_none());
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
            r#"{"stage":"finish","status":"done","summaries":{"finish":"Shipped"},"activity":null,"current_ticket":null}"#,
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
    async fn rework_count_is_set_kept_cleared_and_persisted() {
        let dir = TestDir::new();
        let path = dir.0.path().join("runs.json");
        let api = client(&dir).await;
        let run = api
            .rocket()
            .state::<RunStore>()
            .unwrap()
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
                repository: None,
                session: None,
                model_routing: None,
            })
            .unwrap();
        assert!(run.rework.is_none());
        let response = report(
            &api,
            &run.id,
            r#"{"stage":"build","status":"running","rework":1}"#,
        )
        .await;
        assert_eq!(response.status(), HttpStatus::Ok);
        assert_eq!(response.into_json::<Run>().await.unwrap().rework, Some(1));

        let restarted = client(&dir).await;
        let response = report(
            &restarted,
            &run.id,
            r#"{"stage":"check","status":"running"}"#,
        )
        .await;
        assert_eq!(
            response.into_json::<Run>().await.unwrap().rework,
            Some(1),
            "omitted rework stays unchanged across restart"
        );
        report(
            &restarted,
            &run.id,
            r#"{"stage":"build","status":"running","rework":2}"#,
        )
        .await;
        assert_eq!(load_runs(&path).unwrap()[0].rework, Some(2));
        report(
            &restarted,
            &run.id,
            r#"{"stage":"finish","status":"done","summaries":{"finish":"Shipped"},"rework":null}"#,
        )
        .await;
        assert!(load_runs(&path).unwrap()[0].rework.is_none());
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
                repository: None,
                session: None,
                model_routing: None,
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
            serde_json::json!({"rework": 0}),
            serde_json::json!({"rework": 100}),
            serde_json::json!({"rework": -1}),
            serde_json::json!({"rework": 1.5}),
            serde_json::json!({"rework": "1"}),
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

    async fn post_status(api: &Client, route: &str, token: Option<&str>) -> HttpStatus {
        let body = if route == "/api/runs" {
            r#"{"title":"x","project":"/work"}"#
        } else {
            r#"{"stage":"build","status":"running"}"#
        };
        let mut request = api
            .post(route.to_owned())
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(body);
        if let Some(token) = token {
            request = request.header(Header::new(CLI_TOKEN_HEADER, token.to_owned()));
        }
        request.dispatch().await.status()
    }

    #[rocket::async_test]
    async fn post_requires_the_cli_token_and_refuses_the_page_token() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let cli = api.rocket().state::<CliToken>().unwrap().0.clone();
        let page = session_token(&api).await;
        assert_ne!(cli, page);
        for route in ["/api/runs", "/api/runs/1"] {
            assert_eq!(
                post_status(&api, route, None).await,
                HttpStatus::Unauthorized
            );
            assert_eq!(
                post_status(&api, route, Some(&page)).await,
                HttpStatus::Unauthorized
            );
            assert_eq!(
                post_status(&api, route, Some("wrong")).await,
                HttpStatus::Unauthorized
            );
        }
        assert_eq!(
            post_status(&api, "/api/runs", Some(&cli)).await,
            HttpStatus::Created
        );
        assert!(load_runs(&dir.0.path().join("runs.json"))
            .unwrap()
            .iter()
            .all(|run| run.title == "x"));
    }

    #[rocket::async_test]
    async fn the_token_is_checked_before_the_body_is_parsed() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let post = |route: &'static str, host: &'static str, content_type, token: bool, body| {
            let mut request = api
                .post(route)
                .header(Header::new("Host", host))
                .header(content_type)
                .body(body);
            if token {
                request = request.header(cli_token_header(&api));
            }
            request.dispatch()
        };
        for route in ["/api/runs", "/api/runs/1"] {
            for body in ["{not json", r#"{"unknown":1}"#, ""] {
                let host = "127.0.0.1:4311";
                let no_token = post(route, host, ContentType::JSON, false, body).await;
                assert_eq!(
                    no_token.status(),
                    HttpStatus::Unauthorized,
                    "{route} {body}"
                );
                let evil = "evil.example:4311";
                let bad_shape = post(route, evil, ContentType::JSON, true, body).await;
                assert_eq!(bad_shape.status(), HttpStatus::Forbidden, "{route} {body}");
                let wrong_type = post(route, host, ContentType::Plain, true, body).await;
                assert_eq!(wrong_type.status(), HttpStatus::Forbidden, "{route} {body}");
            }
        }
    }

    #[rocket::async_test]
    async fn the_cli_token_is_in_no_response() {
        let dir = TestDir::new();
        let api = client(&dir).await;
        let cli = api.rocket().state::<CliToken>().unwrap().0.clone();
        let create = api
            .post("/api/runs")
            .header(cli_token_header(&api))
            .header(Header::new("Host", "127.0.0.1:4311"))
            .header(ContentType::JSON)
            .body(r#"{"title":"x","project":"/work"}"#);
        let created = create.dispatch().await;
        assert_eq!(created.status(), HttpStatus::Created);
        let mut bodies = vec![created.into_string().await.unwrap()];
        for route in ["/", "/api/runs", "/api/runs/1", "/api/info"] {
            let response = api
                .get(route)
                .header(Header::new("Host", "127.0.0.1:4311"))
                .dispatch()
                .await;
            assert!(!response.headers().iter().any(|h| h.value().contains(&cli)));
            bodies.push(response.into_string().await.unwrap_or_default());
        }
        assert!(bodies.iter().all(|body| !body.contains(&cli)));
    }

    #[test]
    fn the_token_file_is_named_per_port_next_to_the_runs_file() {
        let runs = Path::new("/data/tds-factory/runs.json");
        assert_eq!(
            token_path(runs, DEFAULT_PORT),
            Path::new("/data/tds-factory/session-token")
        );
        assert_eq!(
            token_path(runs, 4400),
            Path::new("/data/tds-factory/session-token-4400")
        );
    }

    /// A launched server that is shut down and joined on drop, so a failing assertion leaves nothing running.
    struct Launched {
        port: u16,
        shutdown: rocket::Shutdown,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Launched {
        /// Stops the server and waits until its port is free again.
        fn stop(mut self) {
            self.shutdown.clone().notify();
            if let Some(thread) = self.thread.take() {
                thread.join().expect("server thread should not panic");
            }
        }
    }

    impl Drop for Launched {
        fn drop(&mut self) {
            self.shutdown.clone().notify();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// A server thread that is shut down (when a handle exists) and joined on drop. Without a handle the
    /// server never lifted off, so the thread is given a deadline to end by itself.
    struct ServerThread {
        shutdown: Option<rocket::Shutdown>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for ServerThread {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                shutdown.notify();
            }
            let Some(thread) = self.thread.take() else {
                return;
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }

    /// Launches `rocket` on a thread and returns once it has lifted off (with its port), or `None` when it
    /// failed to launch. Panics when it does neither within 10 seconds.
    fn launch(rocket: Rocket<Build>) -> Option<Launched> {
        let quiet = rocket.figment().clone().merge(("log_level", "off"));
        let (sender, receiver) = std::sync::mpsc::channel();
        let rocket = rocket
            .configure(quiet)
            .attach(rocket::fairing::AdHoc::on_liftoff("Up", {
                let sender = sender.clone();
                move |rocket| {
                    let _ = sender.send(Some((rocket.config().port, rocket.shutdown())));
                    Box::pin(async {})
                }
            }));
        let thread = std::thread::spawn(move || {
            // Displaying the error marks it handled; an unhandled launch error panics when dropped.
            if let Err(error) = rocket::execute(rocket.launch()) {
                let _ = error.to_string();
            }
            let _ = sender.send(None);
        });
        match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Some((port, shutdown))) => Some(Launched {
                port,
                shutdown,
                thread: Some(thread),
            }),
            Ok(None) => {
                let _ = thread.join();
                None
            }
            Err(_) => panic!("the server neither lifted off nor failed within 10 seconds"),
        }
    }

    #[test]
    fn each_start_writes_a_new_token_that_a_failed_bind_leaves_alone() {
        let dir = TestDir::new();
        let runs = dir.0.path().join("data").join("runs.json");
        let first = launch(build_rocket(&runs, 0).unwrap()).expect("first start binds");
        let port = first.port;
        let file = token_path(&runs, port);
        let written = fs::read_to_string(&file).unwrap();
        assert_eq!(written.len(), 64);
        assert!(written.bytes().all(|byte| byte.is_ascii_hexdigit()));
        // The port is taken, so this one fails to bind and must not touch the file.
        assert!(launch(build_rocket(&runs, port).unwrap()).is_none());
        assert_eq!(fs::read_to_string(&file).unwrap(), written);
        first.stop();
        let second = launch(build_rocket(&runs, port).unwrap()).expect("restart binds");
        assert_ne!(fs::read_to_string(&file).unwrap(), written);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&file), 0o600);
            assert_eq!(mode(file.parent().unwrap()), 0o700);
        }
        second.stop();
    }

    #[test]
    fn random_tokens_are_64_hex_characters_and_differ() {
        let (a, b) = (random_token(), random_token());
        assert_eq!((a.len(), b.len()), (64, 64));
        assert!(a.bytes().chain(b.bytes()).all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_dashboard_that_cannot_write_its_token_shuts_down_and_says_why() {
        let dir = TestDir::new();
        let data = dir.0.path().join("data");
        let runs = data.join("runs.json");
        let rocket = build_rocket(&runs, 0).unwrap();
        let cli_token = rocket.state::<CliToken>().unwrap().0.clone();
        // The folder for the token turns into a file after the run storage opened.
        fs::remove_dir_all(&data).ok();
        fs::write(&data, "not a folder").unwrap();
        let quiet = rocket.figment().clone().merge(("log_level", "off"));
        let (sender, receiver) = std::sync::mpsc::channel();
        let (up, lifted) = std::sync::mpsc::channel();
        let rocket = rocket
            .configure(quiet)
            .attach(rocket::fairing::AdHoc::on_liftoff("Up", move |rocket| {
                let _ = up.send((rocket.config().port, rocket.shutdown()));
                Box::pin(async {})
            }));
        let thread = std::thread::spawn(move || {
            let ended = rocket::execute(rocket.launch());
            let _ = sender.send(ended.map(|ignited| {
                ignited
                    .state::<TokenWriteFailure>()
                    .and_then(|failure| failure.0.lock().unwrap().take())
            }));
        });
        // Stops and joins the server on every path, including a liftoff that never comes.
        let mut guard = ServerThread {
            shutdown: None,
            thread: Some(thread),
        };
        guard.shutdown = lifted
            .recv_timeout(std::time::Duration::from_secs(10))
            .ok()
            .map(|(_, shutdown)| shutdown);
        let ended = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the server should shut down when the token cannot be written");
        let message = ended.unwrap().expect("the failure is recorded");
        assert!(message.contains("session-token"), "{message}");
        assert!(!message.contains(&cli_token), "{message}");
    }

    #[cfg(unix)]
    #[test]
    fn building_the_server_makes_an_existing_loose_folder_private_before_runs_json_is_written() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let folder = dir.0.path().join("tds-factory");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        let _rocket = build_rocket(&folder.join("runs.json"), 0).unwrap();
        let mode = fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        assert!(!folder.join("runs.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn building_the_server_creates_a_missing_folder_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let folder = dir.0.path().join("tds-factory");
        let _rocket = build_rocket(&folder.join("runs.json"), 0).unwrap();
        let mode = fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_data_folder_keeps_the_mode_of_its_target() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let target = dir.0.path().join("elsewhere");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.0.path().join("tds-factory");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let _rocket = build_rocket(&link.join("runs.json"), 0).unwrap();
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn writing_the_token_creates_a_missing_folder_private_and_ignores_stale_temp_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let folder = dir.0.path().join("tds-factory");
        fs::create_dir(&folder).unwrap();
        let stale = folder.join("session-token.tmp");
        fs::write(&stale, "stale").unwrap();
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o644)).unwrap();
        let path = folder.join("session-token");
        write_token_file(&path, "secret").unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), "secret");
        assert_eq!(fs::read_to_string(&stale).unwrap(), "stale");
        let names: Vec<_> = fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "no temp file is left: {names:?}");
        let fresh = dir.0.path().join("new").join("session-token");
        write_token_file(&fresh, "secret").unwrap();
        assert_eq!(mode(fresh.parent().unwrap()), 0o700);
    }

    #[test]
    fn a_leftover_temp_file_of_a_killed_dashboard_does_not_stop_the_token_write() {
        let dir = TestDir::new();
        let path = dir.0.path().join("session-token");
        let leftovers: Vec<_> = (5..7).map(|n| token_temp_path(&path, n)).collect();
        for leftover in &leftovers {
            fs::write(leftover, "left behind").unwrap();
        }
        write_token_file_from(&path, "secret", 5).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "secret");
        for leftover in &leftovers {
            assert_eq!(fs::read_to_string(leftover).unwrap(), "left behind");
        }
        assert_eq!(fs::read_dir(dir.0.path()).unwrap().count(), 3);
    }

    #[test]
    fn the_token_write_gives_up_after_a_few_taken_temp_names() {
        let dir = TestDir::new();
        let path = dir.0.path().join("session-token");
        for n in 0..TOKEN_TEMP_ATTEMPTS {
            fs::write(token_temp_path(&path, n), "left behind").unwrap();
        }
        let error = write_token_file_from(&path, "secret", 0).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_data_folder_warns_when_its_target_is_open_to_others() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let target = dir.0.path().join("elsewhere");
        fs::create_dir(&target).unwrap();
        let link = dir.0.path().join("tds-factory");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let runs = link.join("runs.json");
        for mode in [0o755, 0o710, 0o704] {
            fs::set_permissions(&target, fs::Permissions::from_mode(mode)).unwrap();
            let warning = secure_data_dir(&runs).unwrap().expect("a warning");
            assert!(warning.contains(&*link.to_string_lossy()), "{warning}");
            assert!(warning.contains("chmod 700"), "{warning}");
        }
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(secure_data_dir(&runs).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_real_data_folder_gives_no_warning() {
        let dir = TestDir::new();
        let runs = dir.0.path().join("tds-factory").join("runs.json");
        assert_eq!(secure_data_dir(&runs).unwrap(), None);
        assert_eq!(secure_data_dir(&runs).unwrap(), None);
    }

    #[test]
    fn a_failed_rename_removes_the_temp_file() {
        let dir = TestDir::new();
        let target = dir.0.path().join("session-token");
        fs::create_dir(&target).unwrap();
        assert!(write_token_file(&target, "secret").is_err());
        let names: Vec<_> = fs::read_dir(dir.0.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "no temp file is left: {names:?}");
    }
}
