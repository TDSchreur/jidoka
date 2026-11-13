mod attention;
mod checks;
mod client;
mod codex_requests;
mod copilot_requests;
mod cost;
mod discovery;
mod doctor;
mod document;
mod gitignore;
mod inspect;
mod install;
mod install_record;
mod plans;
mod pr_open;
mod pr_state;
mod pr_sync;
mod pr_wait;
mod repository;
mod request_logs;
mod requests;
mod requests_pool;
mod routing;
mod run;
mod run_begin;
mod run_check;
mod run_cleanup;
mod run_gate;
mod run_next;
mod server;
mod setup;
mod stages;
mod stats;
mod token;
mod usage;
mod usage_logs;

use client::{fetch_run, fetch_runs, start_run};
use run::UPDATE_HELP;
use server::{DEFAULT_PORT, serve};
use std::{env, io, path::PathBuf, process::ExitCode};

const HELP: &str = "Jidoka\n\nUsage:\n  jidoka install [--agent <codex|claude|copilot>[,...]] [--config-dir <path>]...   (asks which agents when none are given on a terminal; else JIDOKA_AGENT, JIDOKA_CLAUDE_CONFIG_DIRS)\n  jidoka dashboard [--port <port>] [--no-usage]\n  jidoka run stats <id> | --all\n  jidoka pr open <run-id> --title <title> --body-file <path> [--base <branch>] [--json]\n  jidoka pr sync <run-id> [--base <branch>] [--json]\n  jidoka pr wait <run-id> [--timeout <duration>] [--fail-fast] [--json]\n  jidoka run requests <id> | --all [--include-partial] [--exclude <id>]... [--json]\n  jidoka doctor [--agent <codex|claude|copilot>] [--json] [--create-model-routing] [--config-dir <path>]...\n  jidoka run start <title> [--agent <codex|claude|copilot>]\n  jidoka run begin <title> | --run <id> [--agent <codex|claude|copilot>] [--mode <full|balanced|fast|jidoka-full|jidoka-balanced|jidoka-fast>] [--base <branch>] [--branch <name>] [--json]\n  jidoka run inspect <id> [--json]\n  jidoka run check <run-id> [--json]\n  jidoka run next <id> [--json]   (current stage, instructions, open gates, allowed moves, summaries, tickets and routed model)\n  jidoka run gate <id> <gate> <pass|fail> [--check <name>] [--evidence <text>]\n  jidoka run advance <id> [--force --reason <text>]   (--force skips the gates, still follows the stage graph and records the reason)\n  jidoka run sync <run-id> [--json]\n  jidoka run cleanup <run-id>\n  jidoka run update <id> <running|blocked|done> [options]   (never changes the stage; --rework <n> from Check returns to Build)\n  jidoka --help | --version\n\nSetup:\n  `jidoka install` writes an install record (agents and Claude config dirs) next to the model routing policy. A command run before Setup prints a hint to run `jidoka install`; after an upgrade, the first command refreshes the installed skills automatically.";

/// How long the request pool may read logs across all runs: it covers many runs, so it gets the longest wait.
const POOL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

/// How long the request analysis of one run may read logs.
const RUN_REQUESTS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// How long one dashboard usage collection may read logs, so the dashboard stays responsive.
const DASHBOARD_USAGE_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// Names a build: the version for official releases, the commit for any other build.
fn build_label(version: &str, release: bool, commit: &str) -> String {
    if release {
        version.to_string()
    } else if commit.is_empty() {
        "dev (unknown commit)".to_string()
    } else {
        format!("dev {commit}")
    }
}

fn current_build() -> String {
    build_label(
        env!("CARGO_PKG_VERSION"),
        env!("JIDOKA_BUILD_RELEASE") == "true",
        env!("JIDOKA_BUILD_COMMIT"),
    )
}

fn main() -> ExitCode {
    match run_cli() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the command and returns its exit status; a non-zero one means it already reported why.
#[expect(
    clippy::too_many_lines,
    reason = "dispatches every top-level subcommand in one match"
)]
fn run_cli() -> Result<u8, Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if let Ok(home) = install::home_dir() {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        setup::check(&words, |name| env::var_os(name), &home, &mut io::stderr());
    }
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--help" | "-h"] => println!("{HELP}\n\n{UPDATE_HELP}"),
        ["--version" | "-V"] => println!("jidoka {}", current_build()),
        ["install", options @ ..] => install::run(options)?,
        ["doctor", options @ ..] => {
            let args = doctor::parse_args(options).map_err(|error| format!("{error}\n{HELP}"))?;
            return doctor::run(args).map(|ok| u8::from(!ok));
        }
        ["dashboard", options @ ..] => {
            let (port, usage) =
                parse_dashboard_options(options, env::var("JIDOKA_USAGE").ok().as_deref())?;
            dashboard(port, usage)?;
        }
        ["run", "start", title, options @ ..] => {
            let host = routing::parse_start_options(options)
                .map_err(|error| format!("{error}\n{HELP}"))?;
            let dir = env::current_dir()?;
            let repository = repository::canonical_repository(&dir);
            let resolution = resolve_routing(host)?;
            println!(
                "{}",
                start_run(
                    &cli_address()?,
                    &cli_token_file()?,
                    title,
                    &dir.display().to_string(),
                    repository,
                    current_sessions().into_iter().next(),
                    resolution.selection
                )?
            );
        }
        ["run", "begin", "--run", id, options @ ..] => {
            let parsed =
                run_begin::parse_resume(id, options).map_err(|error| format!("{error}\n{HELP}"))?;
            return run_begin_command(parsed);
        }
        ["run", "begin", title, options @ ..] if !title.starts_with('-') => {
            let parsed = run_begin::parse_options(title, options)
                .map_err(|error| format!("{error}\n{HELP}"))?;
            return run_begin_command(parsed);
        }
        ["run", "cleanup", id, options @ ..] if !id.starts_with('-') => {
            return run_cleanup_command(id, options);
        }
        ["run", "check", id, options @ ..] if !id.starts_with('-') => {
            return run_check_command(id, options);
        }
        ["run", "sync", id, options @ ..] if !id.starts_with('-') => {
            let json = match options {
                [] => false,
                ["--json"] => true,
                _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
            };
            return run_sync_command(id, json);
        }
        ["run", "next", id, options @ ..] if !id.starts_with('-') => {
            let json =
                run_next::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
            let run = fetch_run_cli(id)?;
            let paths = if run.stage == run::Stage::Check && run.mode != Some(run::Mode::Fast) {
                run_next::changed_paths(&run, &run_check::ReadOnlyGit)?
            } else {
                Vec::new()
            };
            let next = run_next::build(&run, &paths)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&next)?);
            } else {
                print!("{}", run_next::render(&next));
            }
        }
        ["run", "gate", id, gate, result, options @ ..] if !id.starts_with('-') => {
            if !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Invalid run id".into());
            }
            let gate = serde_json::from_value(serde_json::Value::String((*gate).to_string()))
                .map_err(|_| "Unknown gate")?;
            let passed = match *result {
                "pass" => true,
                "fail" => false,
                _ => return Err("Use pass or fail".into()),
            };
            let (check, evidence) = run_gate::parse_options(options)?;
            let record = run_gate::Record {
                gate,
                passed,
                evidence,
                check,
            };
            client::api_post(
                &cli_address()?,
                &cli_token_file()?,
                &format!("/api/runs/{id}/gate"),
                &serde_json::to_vec(&record)?,
            )?;
            println!("Gate recorded for run {id}");
        }
        ["run", "advance", id, options @ ..] if !id.starts_with('-') => {
            if !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Invalid run id".into());
            }
            let request = match options {
                [] => run_gate::AdvanceRequest::default(),
                ["--force", "--reason", text] => run_gate::AdvanceRequest {
                    force: true,
                    reason: Some((*text).to_string()),
                },
                _ => {
                    return Err(
                        format!("Use --force --reason <text>, or no options.\n{HELP}").into(),
                    );
                }
            };
            request.forced_reason()?;
            client::api_post(
                &cli_address()?,
                &cli_token_file()?,
                &format!("/api/runs/{id}/advance"),
                &serde_json::to_vec(&request)?,
            )?;
            println!("Advanced run {id}");
        }
        ["run", "inspect", id, options @ ..] => {
            let json = match options {
                [] => false,
                ["--json"] => true,
                _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
            };
            let run = fetch_run_cli(id)?;
            let plan = plan_path(id)?.to_string_lossy().into_owned();
            if json {
                let mut shown = serde_json::to_value(&run)?;
                shown["plan"] = plan.into();
                println!("{}", serde_json::to_string_pretty(&shown)?);
            } else {
                print!("{}", inspect::render(&run));
                print!("{}", inspect::plan_line(&plan));
            }
        }
        ["run", "stats", "--all"] => print!("{}", stats::render_all(&fetch_runs(&cli_address()?)?)),
        ["run", "stats", id] if !id.starts_with('-') => {
            print!("{}", stats::render_run(&fetch_run_cli(id)?))
        }
        ["run", "requests", "--all", options @ ..] => {
            let (pool, json) = requests_pool::parse_options(options)
                .map_err(|error| format!("{error}\n{HELP}"))?;
            let runs = fetch_runs(&cli_address()?)?;
            let roots =
                usage_logs::Roots::from_home(&install::home_dir()?, &|name| env::var_os(name))?;
            let result = requests_pool::pool(
                &runs,
                &roots,
                run::updated_now()?,
                std::time::Instant::now() + POOL_DEADLINE,
                &pool,
            );
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", requests_pool::render_pool(&result));
            }
        }
        ["run", "requests", id, options @ ..] if !id.starts_with('-') => {
            let json = match options {
                [] => false,
                ["--json"] => true,
                _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
            };
            let target = fetch_run_cli(id)?;
            let runs = fetch_runs(&cli_address()?)?;
            let roots =
                usage_logs::Roots::from_home(&install::home_dir()?, &|name| env::var_os(name))?;
            let result = requests::analyze(
                &target,
                &runs,
                &roots,
                run::updated_now()?,
                std::time::Instant::now() + RUN_REQUESTS_DEADLINE,
            );
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", requests::render_run(&result));
            }
        }
        ["run", "update", id, status, options @ ..] => {
            // The agent's own session is recorded with every update, so its token usage can be attributed.
            let sessions = current_sessions();
            let mut options = options.to_vec();
            for session in &sessions {
                options.extend(["--session", session]);
            }
            report_to_dashboard(id, status, &options)?;
            println!("Updated run {id}");
        }
        ["pr", "wait", id, options @ ..] if !id.starts_with('-') => {
            return pr_wait_command(id, options);
        }
        ["pr", "open", id, options @ ..] if !id.starts_with('-') => {
            return pr_open_command(id, options);
        }
        ["pr", "sync", id, options @ ..] if !id.starts_with('-') => {
            return pr_sync_command(id, options);
        }
        _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
    }
    Ok(0)
}

impl pr_open::RunApi for ServerApi {
    fn update(&self, id: &str, status: &str, options: &[&str]) -> Result<(), String> {
        report_to_dashboard(id, status, options)
    }
}

/// `jidoka pr open <run-id> --title <title> --body-file <path>`: pushes the run's branch and opens its pull
/// request, or finds the one that is open, and records it on the run.
fn pr_open_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let parsed = pr_open::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    client::check_run_id(id)?;
    let run = fetch_run_cli(id)?;
    let info = pr_open::PrRun {
        id: id.to_string(),
        worktree: run.worktree.clone(),
        branch: run.branch.clone(),
        stage: serde_json::to_value(run.stage)?
            .as_str()
            .unwrap_or("finish")
            .to_string(),
        status: serde_json::to_value(run.status)?
            .as_str()
            .unwrap_or("running")
            .to_string(),
    };
    let opened = pr_open::open(&info, &parsed, &pr_open::RealGh, &ServerApi)
        .map_err(|problem| inspect::shown(&problem.0))?;
    if parsed.json {
        println!("{}", serde_json::to_string_pretty(&opened)?);
    } else {
        print!("{}", pr_open::render(&opened, &info));
    }
    Ok(0)
}

/// `jidoka run check <run-id>`: compares the run's recorded workspace with the repository. It only reads.
/// The exit status is 0 when nothing differs, 2 when something does, and 1 for a problem.
fn run_check_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let json = run_check::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    let run = fetch_run_cli(id)?;
    let recorded = run_check::Recorded {
        id: id.to_string(),
        repository: run.repository.clone(),
        worktree: run.worktree.clone(),
        branch: run.branch.clone(),
        base_commit: run.base_commit.clone(),
    };
    let report = run_check::check(&recorded, &run_check::ReadOnlyGit);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", run_check::render(&report));
    }
    Ok(report.exit_code)
}

/// `jidoka run sync <run-id>`: reads the state of the run's pull request from its host and brings the run in
/// line with it: a merged pull request finishes the run, a closed one blocks it, and an open one changes nothing.
fn run_sync_command(id: &str, json: bool) -> Result<u8, Box<dyn std::error::Error>> {
    client::check_run_id(id)?;
    let run = fetch_run_cli(id)?;
    let Some(url) = run.pr.as_deref() else {
        return Err("The run has no pull request recorded. Record it with `jidoka run update <id> running --pr <url>`.".into());
    };
    let state =
        pr_state::read(&pr_wait::RealGh, url).map_err(|problem| inspect::shown(&problem.0))?;
    let changed = apply_pr_state(id, state)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "run": id,
                "pull_request": url,
                "state": state,
                "run_updated": changed,
            }))?
        );
    } else {
        print!("{}", pr_state::describe(state, changed));
    }
    Ok(0)
}

/// Applies what the state of the run's pull request means for the run (see `pr_state::follow_up`), deciding
/// from the run as it is now: the host may have taken seconds to answer. Returns whether the run was changed.
fn apply_pr_state(id: &str, state: pr_state::PrState) -> Result<bool, Box<dyn std::error::Error>> {
    let run = fetch_run_cli(id)?;
    let Some(update) = pr_state::follow_up(&run, state) else {
        return Ok(false);
    };
    client::send_update(&cli_address()?, &cli_token_file()?, id, &update)?;
    Ok(true)
}

/// `jidoka run cleanup <run-id>`: removes the worktree and local branch of a run whose pull request merged,
/// then clears them from the run. It refuses, removing nothing, when work could be lost.
fn run_cleanup_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    if let Some(other) = options.first() {
        return Err(format!("Unknown option: {other}\n{HELP}").into());
    }
    let run = fetch_run_cli(id)?;
    let recorded = run_cleanup::Recorded {
        id: id.to_string(),
        repository: run.repository.clone(),
        worktree: run.worktree.clone(),
        branch: run.branch.clone(),
        pr: run.pr.clone(),
    };
    let cwd = std::env::current_dir()?;
    let cleaned = run_cleanup::cleanup(&recorded, &pr_wait::RealGh, &pr_sync::RealGit, &cwd)
        .map_err(|problem| inspect::shown(&problem.0))?;
    print!("{}", run_cleanup::render(&recorded, cleaned));
    if cleaned.needs_update() {
        let status = serde_json::to_value(run.status)?
            .as_str()
            .unwrap_or("running")
            .to_string();
        report_to_dashboard(
            id,
            &status,
            &[
                "--clear-worktree",
                "--clear-branch",
                "--activity",
                "Cleaned up worktree and branch",
            ],
        )?;
    }
    Ok(0)
}

/// `jidoka pr sync <run-id>`: brings the run's branch up to date with its base. The exit status is the
/// outcome's; 1 is a problem.
fn pr_sync_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let parsed = pr_sync::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    client::check_run_id(id)?;
    let run = fetch_run_cli(id)?;
    let text =
        |value: serde_json::Value, fallback: &str| value.as_str().unwrap_or(fallback).to_string();
    let info = pr_open::PrRun {
        id: id.to_string(),
        worktree: run.worktree.clone(),
        branch: run.branch.clone(),
        stage: text(serde_json::to_value(run.stage)?, "finish"),
        status: text(serde_json::to_value(run.status)?, "running"),
    };
    let synced = pr_sync::sync(&info, &parsed, &pr_sync::RealGit, &ServerApi)
        .map_err(|problem| inspect::shown(&problem.0))?;
    if parsed.json {
        println!("{}", serde_json::to_string_pretty(&synced)?);
    } else {
        print!("{}", pr_sync::render(&synced));
    }
    Ok(synced.exit_code)
}

/// `jidoka pr wait <run-id>`: watches the CI of the run's pull request and prints the outcome. The exit
/// status is the outcome's; 1 is a problem with the tools or the run.
fn pr_wait_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let (mut wait_options, json) =
        pr_wait::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    client::check_run_id(id)?;
    let gh = pr_wait::RealGh;
    let run = fetch_run_cli(id)?;
    if !run.pr.as_deref().is_some_and(pr_wait::is_azure_pr_url) {
        pr_wait::check_gh(&gh).map_err(|problem| problem.0)?;
    }
    // The run's activity follows the wait, keeping its status. Only a running run is updated.
    let running = matches!(run.status, run::RunStatus::Running);
    let mut reported_failure = false;
    let mut activity = |text: &str| {
        if running
            && let Err(error) = report_to_dashboard(id, "running", &["--activity", text])
            && !reported_failure
        {
            reported_failure = true;
            eprintln!("The run's activity could not be updated: {error}");
        }
    };
    // Only GitHub can lag behind a push in what it reports; the Azure wait does not use the hint.
    let github = !run.pr.as_deref().is_some_and(pr_wait::is_azure_pr_url);
    if let (true, Some(worktree), Some(branch)) = (github, &run.worktree, &run.branch) {
        wait_options.pushed_head = pr_wait::pushed_head(std::path::Path::new(worktree), branch);
    }
    let clock = pr_wait::RealClock(std::time::Instant::now());
    let waited = pr_wait::wait(
        run.pr.as_deref(),
        &wait_options,
        &gh,
        &clock,
        &mut activity,
        &mut |text| eprintln!("{text}"),
    )
    .map_err(|problem| problem.0)?;
    if waited.outcome == pr_wait::Outcome::NotOpen
        && let Some(url) = run.pr.as_deref()
    {
        // The wait only reports that the pull request is gone; the run follows what happened to it.
        match pr_state::read(&gh, url) {
            Ok(state) => match apply_pr_state(id, state) {
                Ok(changed) => eprint!("{}", pr_state::describe(state, changed)),
                Err(error) => eprintln!("The run could not be updated: {error}"),
            },
            Err(problem) => eprintln!("The run was not updated: {}", inspect::shown(&problem.0)),
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&waited)?);
    } else {
        print!("{}", pr_wait::render(&waited));
    }
    Ok(waited.exit_code)
}

/// Reads the model-routing policy once, before a run exists: a mistake in it, or a policy for several
/// agents without `--agent`, stops the start. Prints the notice, if any.
fn resolve_routing(host: Option<routing::Host>) -> Result<routing::Resolution, String> {
    let resolution = routing::policy_path_in(|name| env::var_os(name)).map_or_else(
        || Ok(routing::Resolution::default()),
        |path| routing::load(&path, host).map_err(|error| inspect::shown(&error)),
    )?;
    if let Some(notice) = &resolution.notice {
        eprintln!("{notice}");
    }
    Ok(resolution)
}

/// The real jidoka server for `jidoka run begin`.
struct ServerApi;

impl run_begin::RunApi for ServerApi {
    fn create(
        &self,
        title: &str,
        project: &str,
        repository: Option<String>,
        session: Option<String>,
        routing: Option<routing::ModelRouting>,
    ) -> Result<String, String> {
        let token = cli_token_file().map_err(|e| e.to_string())?;
        start_run(
            &cli_address()?,
            &token,
            title,
            project,
            repository,
            session,
            routing,
        )
        .map_err(|e| e.to_string())
    }

    fn get(&self, id: &str) -> Result<run_begin::RunSummary, String> {
        let run = fetch_run_cli(id).map_err(|e| e.to_string())?;
        Ok(run_begin::RunSummary {
            title: run.title,
            finished: matches!(run.status, run::RunStatus::Done),
            worktree: run.worktree,
        })
    }

    fn update(&self, id: &str, status: &str, options: &[&str]) -> Result<(), String> {
        report_to_dashboard(id, status, options)
    }

    fn runs_with_session(&self, session: &str) -> Result<Vec<(String, String)>, String> {
        let runs = fetch_runs(&cli_address()?).map_err(|e| e.to_string())?;
        Ok(runs
            .into_iter()
            .filter(|run| run.sessions.iter().any(|known| known == session))
            .map(|run| (run.id, run.title))
            .collect())
    }
}

/// `jidoka run begin <title>`, or `--run <id>` to resume: creates a run and its workspace.
fn run_begin_command(parsed: run_begin::Options) -> Result<u8, Box<dyn std::error::Error>> {
    let host = parsed.agent.as_deref().and_then(routing::Host::parse);
    let dir = env::current_dir()?;
    // A resumed run already has its model routing; the policy is read only when a run is created.
    let resolution = if parsed.run.is_some() {
        routing::Resolution::default()
    } else {
        resolve_routing(host)?
    };
    match run_begin::begin(
        &dir,
        &parsed,
        &ServerApi,
        current_sessions().into_iter().next(),
        resolution.selection,
    ) {
        Ok(begun) => {
            let plan = plan_path(&begun.run)?.to_string_lossy().into_owned();
            if parsed.json {
                let shown = run_begin::Output {
                    begun: &begun,
                    plan,
                };
                println!("{}", serde_json::to_string_pretty(&shown)?);
            } else {
                print!("{}", run_begin::render(&begun, &plan));
            }
            Ok(0)
        }
        Err(run_begin::BeginError::NoRun(text)) => Err(inspect::shown(&text).into()),
        Err(run_begin::BeginError::Blocked {
            run,
            reason,
            retry,
            marked,
        }) => {
            let state = if marked {
                "is blocked"
            } else {
                "could not be marked blocked (the server did not accept the update)"
            };
            eprintln!(
                "Run {run} {state}: {}\nTo retry: {}",
                inspect::shown(&reason),
                inspect::shown(&retry)
            );
            Ok(1)
        }
    }
}

/// The ids of the agent sessions this command runs in, when the agent exposes them: Claude Code sets
/// `CLAUDE_CODE_SESSION_ID`; Codex sets `CODEX_SESSION_ID` (the root session) and `CODEX_THREAD_ID`; Copilot CLI sets `COPILOT_AGENT_SESSION_ID`.
fn current_sessions() -> Vec<String> {
    sessions_from(&|name| env::var_os(name))
}

fn sessions_from(var: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Vec<String> {
    let mut sessions: Vec<String> = Vec::new();
    for name in [
        "CLAUDE_CODE_SESSION_ID",
        "CODEX_SESSION_ID",
        "CODEX_THREAD_ID",
        "COPILOT_AGENT_SESSION_ID",
    ] {
        let Some(id) = var(name).and_then(|value| value.into_string().ok()) else {
            continue;
        };
        if run::check_session_id(&id).is_ok() && !sessions.contains(&id) {
            sessions.push(id);
        }
    }
    sessions
}

/// `[--port <port>] [--no-usage]` in any order. Usage collection is on unless `--no-usage` or `JIDOKA_USAGE=off`.
fn parse_dashboard_options(
    options: &[&str],
    jidoka_usage: Option<&str>,
) -> Result<(Option<u16>, bool), String> {
    let (mut port, mut usage) = (None, true);
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--port" if port.is_none() => {
                port = Some(parse_port(
                    args.next().ok_or("--port needs a port number.")?,
                )?);
            }
            "--no-usage" => usage = false,
            other => return Err(format!("Invalid dashboard option {other:?}.\n{HELP}")),
        }
    }
    if usage::switched_off_by(jidoka_usage) {
        usage = false;
    }
    Ok((port, usage))
}

fn parse_port(port: &str) -> Result<u16, String> {
    match port.parse::<u16>() {
        Ok(port) if port != 0 => Ok(port),
        _ => Err(format!(
            "Invalid port '{port}': use a number from 1 to 65535."
        )),
    }
}

/// Escapes everything but unreserved characters, so a path can sit in a query string.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The dashboard URL that opens on `project`: a Project's id is its path.
fn dashboard_url(port: u16, project: &str) -> String {
    format!(
        "http://127.0.0.1:{port}/?project={}",
        percent_encode(project)
    )
}

/// The port to bind. An explicit port must be free, else this fails. Without one, `default` if it is free, else 0,
/// which lets the operating system pick a free port.
fn choose_port(requested: Option<u16>, default: u16) -> Result<u16, String> {
    let is_free = |port| std::net::TcpListener::bind((server::LOOPBACK_HOST, port)).is_ok();
    match requested {
        Some(port) if is_free(port) => Ok(port),
        Some(port) => Err(format!(
            "Port {port} is already in use. Pick another with 'jidoka dashboard --port <port>', or leave --port out to let Jidoka choose a free port."
        )),
        None if is_free(default) => Ok(default),
        None => Ok(0),
    }
}

/// Serves the dashboard, or, when a Jidoka dashboard is already running, prints its URL.
fn dashboard(requested: Option<u16>, usage: bool) -> Result<(), Box<dyn std::error::Error>> {
    // A Project is identified by its repository, so a linked worktree opens its main checkout's row.
    let dir = env::current_dir()?;
    let project =
        repository::canonical_repository(&dir).unwrap_or_else(|| dir.display().to_string());
    let runs = runs_path()?;
    if let Some(running) = discovery::running(server::data_dir(&runs)) {
        println!(
            "Dashboard already running (pid {}): {}",
            running.pid,
            dashboard_url(running.port, &project)
        );
        if !usage {
            eprintln!(
                "Note: --no-usage and JIDOKA_USAGE only apply to a dashboard this command starts."
            );
        }
        let ours = current_build();
        if running.version != ours {
            eprintln!(
                "Warning: the running dashboard is jidoka {}, this is jidoka {ours}. It was left running; stop it and run 'jidoka dashboard' again to use this version.",
                running.version
            );
        }
        return Ok(());
    }
    let port = choose_port(requested, DEFAULT_PORT)?;
    let home = install::home_dir()?;
    let settings = usage::UsageSettings {
        enabled: usage,
        roots: usage_logs::Roots::from_home(&home, &|name| env::var_os(name))?,
        budget: DASHBOARD_USAGE_BUDGET,
        settle: usage::UsageSettings::default_settle(),
    };
    serve(
        &runs,
        port,
        move |bound| dashboard_url(bound, &project),
        settings,
    )?;
    Ok(())
}

/// Reports a run update to the running dashboard, with the CLI token it wrote. A bad id or option is refused
/// before the dashboard is looked for.
fn report_to_dashboard(id: &str, status: &str, options: &[&str]) -> Result<(), String> {
    let update = client::prepare_update(id, status, options).map_err(|e| e.to_string())?;
    let token_file = cli_token_file().map_err(|e| e.to_string())?;
    client::send_update(&cli_address()?, &token_file, id, &update).map_err(|e| e.to_string())
}

/// Reads run `id` from the dashboard; a bad id is refused before the dashboard is looked for.
fn fetch_run_cli(id: &str) -> Result<run::Run, Box<dyn std::error::Error>> {
    client::check_run_id(id)?;
    fetch_run(&cli_address()?, id)
}

/// The server the CLI talks to: the dashboard `dashboard.json` points to, or an error saying to start one. Debug
/// builds honor `JIDOKA_TEST_ADDRESS`, so CLI tests can use a fake server on an OS-assigned port.
fn cli_address() -> Result<String, String> {
    #[cfg(debug_assertions)]
    if let Ok(address) = env::var("JIDOKA_TEST_ADDRESS") {
        return Ok(address);
    }
    let runs = runs_path().map_err(|error| error.to_string())?;
    discovery::running(server::data_dir(&runs))
        .map(|running| format!("{}:{}", server::LOOPBACK_HOST, running.port))
        .ok_or_else(|| {
            "no Jidoka dashboard is running; start it with 'jidoka dashboard'".to_string()
        })
}

/// Where the dashboard writes the CLI token that POSTs must send.
fn cli_token_file() -> io::Result<PathBuf> {
    Ok(server::token_path(&runs_path()?))
}

/// Where the plan of run `id` is saved, in the data dir; its folder is created.
fn plan_path(id: &str) -> io::Result<PathBuf> {
    plans::plan_path_ready(&runs_path()?, id)
}

/// The dashboard's `runs.json`; its folder also holds the CLI token, the plans and `dashboard.json`.
fn runs_path() -> io::Result<PathBuf> {
    runs_path_in(&|name| env::var_os(name))
}

fn runs_path_in(var: &impl Fn(&str) -> Option<std::ffi::OsString>) -> io::Result<PathBuf> {
    #[cfg(windows)]
    let base = var("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let base = base.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot determine the user data directory",
        )
    })?;
    Ok(base.join("jidoka").join("runs.json"))
}

#[cfg(test)]
mod test_support {
    pub(crate) struct TestDir(pub(crate) tempfile::TempDir);
    impl TestDir {
        pub(crate) fn new() -> Self {
            Self(tempfile::tempdir().unwrap())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_every_agent_wherever_it_names_one() {
        let expected = crate::install::AGENTS.join("|");
        let mut groups = 0;
        for group in HELP.split('<').skip(1) {
            let group = group.split('>').next().unwrap();
            if crate::install::AGENTS
                .iter()
                .any(|name| group.contains(name))
            {
                assert_eq!(group, expected);
                groups += 1;
            }
        }
        assert!(groups > 0);
    }

    fn lookup<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).into())
        }
    }

    #[test]
    fn codex_sessions_are_recorded_from_the_environment() {
        let id = "019e1f00-aaaa-7bbb-8ccc-000000000001";
        let thread = "019e1f00-aaaa-7bbb-8ccc-000000000002";
        let both = [("CODEX_SESSION_ID", id), ("CODEX_THREAD_ID", thread)];
        assert_eq!(sessions_from(&lookup(&both)), [id, thread]);
        let same = [("CODEX_SESSION_ID", id), ("CODEX_THREAD_ID", id)];
        assert_eq!(sessions_from(&lookup(&same)), [id]);
        let thread_only = [("CODEX_THREAD_ID", thread)];
        assert_eq!(sessions_from(&lookup(&thread_only)), [thread]);
        let claude = [("CLAUDE_CODE_SESSION_ID", id)];
        assert_eq!(sessions_from(&lookup(&claude)), [id]);
        let invalid = [("CODEX_SESSION_ID", "bad id!"), ("CODEX_THREAD_ID", "x")];
        let copilot = [("COPILOT_AGENT_SESSION_ID", id)];
        assert_eq!(sessions_from(&lookup(&copilot)), [id]);
        assert!(sessions_from(&lookup(&invalid)).is_empty());
        assert!(sessions_from(&lookup(&[])).is_empty());
    }

    #[test]
    fn ports_are_numbers_from_1_to_65535() {
        assert_eq!(parse_port("4400"), Ok(4400));
        for bad in ["0", "65536", "-1", "abc", ""] {
            assert!(parse_port(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_default_port_is_used_when_free_and_a_free_one_when_taken() {
        let taken = std::net::TcpListener::bind((server::LOOPBACK_HOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert_eq!(choose_port(None, port), Ok(0));
    }

    #[test]
    fn a_requested_port_that_is_taken_is_an_error() {
        let taken = std::net::TcpListener::bind((server::LOOPBACK_HOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let error = choose_port(Some(port), DEFAULT_PORT).unwrap_err();
        assert!(error.contains("already in use"), "{error}");
    }

    #[test]
    fn dashboard_options_set_the_port_and_the_usage_switch_in_any_order() {
        assert_eq!(parse_dashboard_options(&[], None), Ok((None, true)));
        assert_eq!(
            parse_dashboard_options(&["--no-usage", "--port", "4400"], None),
            Ok((Some(4400), false))
        );
        assert_eq!(
            parse_dashboard_options(&["--port", "4400"], Some("off")),
            Ok((Some(4400), false))
        );
        for on in [None, Some(""), Some("on"), Some("1"), Some("true")] {
            assert!(parse_dashboard_options(&[], on).unwrap().1, "{on:?}");
        }
        for off in ["off", "OFF", " false ", "0", "no"] {
            assert!(!parse_dashboard_options(&[], Some(off)).unwrap().1, "{off}");
        }
        for bad in [
            &["--port"][..],
            &["--port", "0"],
            &["--port", "1", "--port", "2"],
            &["--nope"],
            &["4400"],
        ] {
            assert!(parse_dashboard_options(bad, None).is_err(), "{bad:?}");
        }
    }
}
