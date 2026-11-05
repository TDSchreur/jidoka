use crate::{
    current_build,
    run::{NewRun, Run, RunStore, UpdateRun},
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
use std::{io, path::Path};

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
];
/// Placeholder in the dashboard page that is replaced with the build label.
const BUILD_PLACEHOLDER: &str = "__BUILD__";
const BODY_LIMIT: u64 = 64 * 1024;

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
fn dashboard(request: Security) -> Result<RawHtml<String>, HttpStatus> {
    if !request.local {
        return Err(HttpStatus::BadRequest);
    }
    Ok(RawHtml(
        DASHBOARD.replace(BUILD_PLACEHOLDER, &current_build()),
    ))
}

#[rocket::get("/assets/<path..>")]
fn asset(
    request: Security,
    path: std::path::PathBuf,
) -> Result<(ContentType, &'static [u8]), HttpStatus> {
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

pub(crate) fn build_rocket(path: &Path, port: u16) -> io::Result<Rocket<Build>> {
    let config = Figment::from(rocket::Config::default())
        .merge(("address", "127.0.0.1"))
        .merge(("port", port))
        .merge(("limits", Limits::new().limit("json", BODY_LIMIT.bytes())));
    Ok(rocket::custom(config)
        .manage(RunStore::open(path)?)
        .mount(
            "/",
            rocket::routes![dashboard, asset, list_runs, create_run, update_run],
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

pub(crate) fn serve(path: &Path) -> io::Result<()> {
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
        Client::tracked(build_rocket(&dir.0.path().join("runs.json"), 0).unwrap())
            .await
            .unwrap()
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
            page.contains("Needs you (${count})"),
            "missing Needs you toggle"
        );
        assert!(
            page.contains("aria-pressed"),
            "the toggle reports its state"
        );
        assert!(page.contains("document.title"), "the count is in the title");
        assert!(page.contains("m ago"), "missing relative time");
        assert!(
            page.contains("title=${date.toLocaleString()}"),
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
    }

    async fn stylesheet() -> String {
        let dir = TestDir::new();
        let api = client(&dir).await;
        api.get("/assets/style.css")
            .header(Header::new("Host", "127.0.0.1:4311"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap()
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
    async fn dashboard_board_has_a_sticky_column_per_stage_on_wide_screens_only() {
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        let (stacked, wide) = rules
            .split_once("@media (min-width: 1025px)")
            .expect("a wide-screen layout");
        let wide = &wide[..wide
            .find("@media (prefers-reduced-motion")
            .unwrap_or(wide.len())];
        // Narrow screens keep the plain stacked groups.
        assert!(!stacked.contains("grid-template-columns"));
        assert!(!stacked.contains("position: sticky"));
        assert!(
            wide.contains("position: sticky"),
            "the column header sticks"
        );
        // One column for each group the page renders.
        let script = {
            let dir = TestDir::new();
            client(&dir)
                .await
                .get("/assets/app.js")
                .header(Header::new("Host", "127.0.0.1:4311"))
                .dispatch()
                .await
                .into_string()
                .await
                .unwrap()
        };
        let groups = script
            .split("const GROUPS = [")
            .nth(1)
            .and_then(|rest| rest.split("];").next())
            .unwrap()
            .lines()
            .filter(|line| line.trim_start().starts_with("['"))
            .count();
        assert!(
            wide.contains(&format!("grid-template-columns: repeat({groups},")),
            "{groups} columns"
        );
        assert!(script.contains("class=\"group-header\""));
    }

    #[rocket::async_test]
    async fn dashboard_shows_one_stage_at_a_time_up_to_1024_px_and_all_stages_above() {
        let css = stylesheet().await;
        let (_, rules) = split_tokens(&css);
        // Compact: the groups other than the picked Stage are hidden up to 1024 px.
        let compact = rules
            .split_once("@media (max-width: 1024px)")
            .expect("a compact layout")
            .1;
        let compact = &compact[..compact.find('}').unwrap() + 1];
        assert!(compact.contains(".group:not(.selected) { display: none; }"));
        // Wide: every column shows, so the Stage buttons go away.
        let wide = rules
            .split_once("@media (min-width: 1025px)")
            .expect("a wide layout")
            .1;
        assert!(wide.contains(".stages { display: none; }"));
        let script = asset("/assets/app.js").await;
        assert!(script.contains("aria-label=\"Stage\""), "a labelled group");
        assert!(script.contains("aria-pressed=${String(stage.id === current)}"));
        assert!(
            script.contains("needs you"),
            "a text alternative for the marker"
        );
        // The Stage is chosen once from the first answer, then only the visitor changes it.
        assert!(script.contains("if (runs && stage === null) setStage(firstStage(stages))"));
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
            r#"{"stage":"finish","status":"done","rework":null}"#,
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
}
