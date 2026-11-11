use super::*;
use crate::{
    run::{Stage, load_runs},
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
            percent_encoding::utf8_percent_encode(reference, percent_encoding::NON_ALPHANUMERIC)
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
async fn traversal_out_of_the_roots_is_rejected() {
    let (dir, _, _, id) = document_setup("../../outside.md", &[]);
    fs::write(dir.0.path().join("outside.md"), "secret").unwrap();
    let (status, _, _) = fetch_document(&dir, &id, "../../outside.md").await;
    assert_eq!(status, HttpStatus::Forbidden);
}

// Creating symlinks on Windows needs extra privileges.
#[rocket::async_test]
async fn only_markdown_files_are_served() {
    let (dir, _, worktree, id) = document_setup("notes.txt", &[]);
    fs::write(worktree.join("notes.txt"), "text").unwrap();
    let (status, _, _) = fetch_document(&dir, &id, "notes.txt").await;
    assert_eq!(status, HttpStatus::BadRequest);
}

/// A run with a plan file written in the plans folder beside `runs.json`, recorded as its spec.
fn plan_run(dir: &TestDir, own: bool) -> (String, String) {
    let worktree = dir.0.path().join("wt");
    let runs = dir.0.path().join("runs.json");
    let repository = dir.0.path().join("repo");
    let id = document_run(&runs, &worktree, &repository, "x.md", &[]);
    let other = document_run(&runs, &worktree, &repository, "x.md", &[]);
    let plans = crate::plans::plans_dir(&runs, DEFAULT_PORT);
    fs::create_dir_all(&plans).unwrap();
    fs::write(plans.join(format!("{id}.md")), "own plan").unwrap();
    fs::write(plans.join(format!("{other}.md")), "other plan").unwrap();
    let wanted = if own { &id } else { &other };
    let plan = plans
        .join(format!("{wanted}.md"))
        .to_string_lossy()
        .into_owned();
    RunStore::open(&runs)
        .unwrap()
        .update(
            &id,
            serde_json::from_value(
                serde_json::json!({ "stage": "build", "status": "running", "spec": plan }),
            )
            .unwrap(),
        )
        .unwrap();
    (id, plan)
}

#[rocket::async_test]
async fn the_runs_own_plan_is_served_without_a_worktree() {
    let dir = TestDir::new();
    let (id, plan) = plan_run(&dir, true);
    let (status, body, _) = fetch_document(&dir, &id, &plan).await;
    assert_eq!(status, HttpStatus::Ok, "{body}");
    assert!(body["html"].as_str().unwrap().contains("own plan"));
}

#[rocket::async_test]
async fn another_runs_plan_is_refused_even_when_recorded() {
    let dir = TestDir::new();
    let (id, plan) = plan_run(&dir, false);
    let (status, body, _) = fetch_document(&dir, &id, &plan).await;
    assert_eq!(status, HttpStatus::Forbidden, "{body}");
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
    let info: DashboardIdentity = response.into_json().await.unwrap();
    assert_eq!(info.pid, std::process::id());
    assert_eq!(info.version, current_build());
    let foreign = api
        .get("/api/info")
        .header(Header::new("Host", "attacker.example"))
        .dispatch()
        .await;
    assert_eq!(foreign.status(), HttpStatus::BadRequest);
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
fn claude_fixture(home: &Path, session: &str, run_id: &str, response: &str, ts: u64, input: u64) {
    let line = serde_json::json!({"type": "assistant", "timestamp": crate::usage::iso_timestamp(ts), "requestId": response,
        "message": {"id": response, "model": "claude-test", "usage": {"input_tokens": input, "output_tokens": 1}}});
    let mention = serde_json::json!({"type": "user", "message": {"content": format!("jidoka run update {run_id}")}});
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
    assert!(
        response
            .into_string()
            .await
            .unwrap()
            .contains("--summary finish=")
    );
    let stored = fetch(&api, &run.id).await.1.unwrap();
    assert!(stored.finished_at.is_none());
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

    let body = r#"{"stage":"build","status":"running","mode":"jidoka-full","branch":"feature/x",
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
async fn starting_a_run_validates_its_model_routing_selection() {
    let dir = TestDir::new();
    let api = client(&dir).await;
    let with =
        |routing: &str| format!(r#"{{"title":"t","project":"/work","model_routing":{routing}}}"#);
    let long = "m".repeat(65);
    for routing in [
        // The earlier shape, with roles and tasks.
        r#"{"agent":"claude","roles":{"strong":{"model":"sonnet"}},"tasks":{"check":"strong"}}"#
            .to_string(),
        // Empty, oversized and unsafe text.
        r#"{"agent":"claude","stages":{"check":{"model":""}}}"#.to_string(),
        format!(r#"{{"agent":"claude","stages":{{"check":{{"model":"{long}"}}}}}}"#),
        r#"{"agent":"claude","stages":{"check":{"model":"son net"}}}"#.to_string(),
        r#"{"agent":"claude","stages":{"check":{"model":"sonnet","effort":"hi\u001b[2Jgh"}}}"#
            .to_string(),
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
    assert!(
        api.rocket()
            .state::<RunStore>()
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
    let run = start(
        &api,
        &with(r#"{"agent":"codex","stages":{"check":{"model":"gpt-6-astra","effort":"high"}}}"#),
    )
    .await;
    let routing = run.model_routing.unwrap();
    assert_eq!(routing.stages.len(), 1);
}

#[rocket::async_test]
async fn ticket_reports_are_recorded_and_rejected_atomically_via_api() {
    let dir = TestDir::new();
    let api = client(&dir).await;
    let run = start(&api, r#"{"title":"t","project":"/work"}"#).await;
    let body = r#"{"stage":"build","status":"running","tickets":[{"reference":"local=42","title":"One"}],"ticket_reports":[{"ticket":"local=42","kind":"skill","text":"jidoka-tdd"}]}"#;
    assert_eq!(report(&api, &run.id, body).await.status(), HttpStatus::Ok);
    let bad = r#"{"stage":"build","status":"running","ticket_reports":[{"ticket":"missing","kind":"activity","text":"oops"}]}"#;
    assert_eq!(
        report(&api, &run.id, bad).await.status(),
        HttpStatus::BadRequest
    );
    let saved = fetch(&api, &run.id).await.1.unwrap();
    assert_eq!(saved.ticket_reports.len(), 1);
    assert_eq!(saved.ticket_reports[0].text, "jidoka-tdd");
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

async fn client_with_pr_tool(
    dir: &TestDir,
    tool: std::sync::Arc<dyn crate::pr_wait::Gh + Send + Sync>,
) -> Client {
    Client::tracked(
        build_rocket_with_tool(&dir.0.path().join("runs.json"), DEFAULT_PORT, tool).unwrap(),
    )
    .await
    .unwrap()
}

fn waiting_run(api: &Client, pr: Option<&str>) -> String {
    let store = api.rocket().state::<RunStore>().unwrap();
    let run = store
        .create(NewRun {
            title: "t".into(),
            project: "/work".into(),
            repository: None,
            session: None,
            model_routing: None,
        })
        .unwrap();
    let pr = pr.map_or(String::new(), |pr| format!(r#","pr":"{pr}""#));
    let update =
        format!(r#"{{"stage":"finish","status":"running","activity":"Waiting for CI"{pr}}}"#);
    store
        .update(&run.id, serde_json::from_str(&update).unwrap())
        .unwrap();
    run.id
}

async fn sync<'a>(
    api: &'a Client,
    id: &str,
    token: Option<&str>,
) -> rocket::local::asynchronous::LocalResponse<'a> {
    let mut request = api
        .post(format!("/api/runs/{id}/sync"))
        .header(Header::new("Host", "127.0.0.1:4311"));
    if let Some(token) = token {
        request = request.header(Header::new(TOKEN_HEADER, token.to_string()));
    }
    request.dispatch().await
}

#[rocket::async_test]
async fn syncing_a_run_reads_its_pull_request_and_follows_what_happened_to_it() {
    let dir = TestDir::new();
    let api = client_with_pr_tool(&dir, std::sync::Arc::new(MergedGh)).await;
    let id = waiting_run(&api, Some("https://github.com/o/r/pull/7"));
    let token = session_token(&api).await;
    let response = sync(&api, &id, Some(&token)).await;
    assert_eq!(response.status(), HttpStatus::Ok);
    let body: serde_json::Value = response.into_json().await.unwrap();
    assert_eq!(body["state"], "merged");
    let run = &api.rocket().state::<RunStore>().unwrap().list().unwrap()[0];
    assert_eq!(run.status, crate::run::RunStatus::Done);
    assert_eq!(run.activity, None);
}

#[rocket::async_test]
async fn syncing_needs_the_session_token_and_a_run_with_a_pull_request() {
    let dir = TestDir::new();
    let api = client_with_pr_tool(&dir, std::sync::Arc::new(MergedGh)).await;
    let with_pr = waiting_run(&api, Some("https://github.com/o/r/pull/7"));
    let without_pr = waiting_run(&api, None);
    let token = session_token(&api).await;
    assert_eq!(
        sync(&api, &with_pr, None).await.status(),
        HttpStatus::Forbidden
    );
    assert_eq!(
        sync(&api, &with_pr, Some("wrong")).await.status(),
        HttpStatus::Forbidden
    );
    assert_eq!(
        sync(&api, &without_pr, Some(&token)).await.status(),
        HttpStatus::NotFound
    );
    assert_eq!(
        sync(&api, "999", Some(&token)).await.status(),
        HttpStatus::NotFound
    );
    assert_eq!(
        sync(&api, "abc", Some(&token)).await.status(),
        HttpStatus::BadRequest
    );
    let runs = api.rocket().state::<RunStore>().unwrap().list().unwrap();
    assert!(
        runs.iter()
            .all(|run| run.activity.as_deref() == Some("Waiting for CI"))
    );
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
async fn each_server_start_has_its_own_session_token() {
    let (first, second) = (TestDir::new(), TestDir::new());
    let (first, second) = (client(&first).await, client(&second).await);
    let (a, b) = (session_token(&first).await, session_token(&second).await);
    assert_eq!(a.len(), 64);
    assert_ne!(a, b);
    assert_eq!(a, session_token(&first).await);
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
async fn update_response_carries_warnings_beside_the_run() {
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    fs::write(
        &path,
        r#"[{"id":"42","title":"old","gates":[],"project":"/work","stage":"define","status":"running"}]"#,
    )
    .unwrap();
    let api = client(&dir).await;
    let response = report(&api, "42", r#"{"stage":"build","status":"running"}"#).await;
    let body: serde_json::Value = response.into_json().await.unwrap();
    assert_eq!(body["warnings"], serde_json::json!([]));
    assert_eq!(body["id"], "42");
    let response = report(
        &api,
        "42",
        r#"{"stage":"finish","status":"done","summaries":{"finish":"Shipped"}}"#,
    )
    .await;
    assert_eq!(response.status(), HttpStatus::Ok);
    let body: serde_json::Value = response.into_json().await.unwrap();
    let warnings = body["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("never entered"));
    assert_eq!(body["stage"], "finish");
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
    assert!(
        load_runs(&dir.0.path().join("runs.json"))
            .unwrap()
            .iter()
            .all(|run| run.title == "x")
    );
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

#[rocket::async_test]
async fn run_api_exposes_gate_status_and_skipped_checks() {
    let dir = TestDir::new();
    fs::write(dir.0.path().join("runs.json"), r#"[{"id":"42","title":"t","project":"/work","mode":"jidoka-fast","stage":"build","status":"running","gates":[{"gate":"verified","stage":"build","rework":null,"passed":false,"evidence":"Tests failed","commit":null,"ticket":null}]}]"#).unwrap();
    let api = client(&dir).await;
    let response = api
        .get("/api/runs/42")
        .header(Header::new("Host", "127.0.0.1:4311"))
        .dispatch()
        .await;
    assert_eq!(response.status(), HttpStatus::Ok);
    let body: serde_json::Value =
        serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
    let gates = body["stage_gates"].as_array().unwrap();
    let build = gates.iter().find(|s| s["stage"] == "build").unwrap();
    let verified = build["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["name"] == "verified")
        .unwrap();
    assert_eq!(verified["status"], "failed");
    assert_eq!(verified["evidence"], "Tests failed");
    assert_eq!(body["checks"]["skipped"][0]["name"], "code-review");
    assert!(
        body["checks"]["skipped"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("no check stage")
    );
    let finish = gates.iter().find(|s| s["stage"] == "finish").unwrap();
    assert_eq!(finish["gates"][0]["status"], "unknown");
}
