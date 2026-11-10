mod client;
mod codex_requests;
mod copilot_requests;
mod cost;
mod doctor;
mod document;
mod gitignore;
mod inspect;
mod install;
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
mod run_reinstall;
mod server;
mod stats;
mod token;
mod update;
mod usage;
mod usage_logs;

use client::{Probe, fetch_run, fetch_runs, probe_dashboard, report_run, start_run};
use run::UPDATE_HELP;
use server::{ADDRESS, DEFAULT_PORT, serve};
use std::{env, io, path::PathBuf, process::ExitCode};

const HELP: &str = "TDS Factory\n\nUsage:\n  factory install --agent <codex|claude|copilot>[,...] [--config-dir <path>]...\n  factory dashboard [--port <port>] [--no-usage]\n  factory run stats <id> | --all\n  factory pr open <run-id> --title <title> --body-file <path> [--base <branch>] [--json]\n  factory pr sync <run-id> [--base <branch>] [--json]\n  factory pr wait <run-id> [--timeout <duration>] [--fail-fast] [--json]\n  factory run requests <id> | --all [--include-partial] [--exclude <id>]... [--json]\n  factory check\n  factory update [--agent <codex|claude|copilot>[,...]] [--config-dir <path>]...\n  factory doctor [--agent <codex|claude|copilot>] [--json] [--create-model-routing] [--config-dir <path>]...\n  factory run start <title> [--agent <codex|claude|copilot>]\n  factory run begin <title> | --run <id> [--agent <codex|claude|copilot>] [--mode <factory-full|factory-balanced|factory-fast>] [--base <branch>] [--branch <name>] [--json]\n  factory run inspect <id> [--json]\n  factory run check <run-id> [--json]\n  factory run sync <run-id> [--json]\n  factory run cleanup <run-id> [--reinstall] [--config-dir <path>]...\n  factory run update <id> <define|build|check|finish> <running|blocked|done> [options]\n  factory --help | --version";

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
        env!("FACTORY_BUILD_RELEASE") == "true",
        env!("FACTORY_BUILD_COMMIT"),
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
fn run_cli() -> Result<u8, Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--help" | "-h"] => println!("{HELP}\n\n{UPDATE_HELP}"),
        ["--version" | "-V"] => println!("factory {}", current_build()),
        ["install", options @ ..] => match install::parse_options(options) {
            Ok((Some(agent), dirs)) => install::install(agent, &dirs)?,
            Ok((None, _)) => return Err(format!("install needs --agent.\n{HELP}").into()),
            Err(error) => return Err(format!("{error}\n{HELP}").into()),
        },
        ["update", options @ ..] => {
            let (agents, dirs) =
                install::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
            update::update(
                env!("CARGO_PKG_VERSION"),
                env!("FACTORY_BUILD_RELEASE") == "true",
                agents,
                &dirs,
            )?
        }
        ["check"] => update::check(
            env!("CARGO_PKG_VERSION"),
            env!("FACTORY_BUILD_RELEASE") == "true",
        )?,
        ["doctor", options @ ..] => {
            let args = doctor::parse_args(options).map_err(|error| format!("{error}\n{HELP}"))?;
            return doctor::run(args).map(|ok| u8::from(!ok));
        }
        ["dashboard", options @ ..] => {
            let (port, usage) =
                parse_dashboard_options(options, env::var("FACTORY_USAGE").ok().as_deref())?;
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
                    ADDRESS,
                    &cli_token_file()?,
                    title,
                    &dir.display().to_string(),
                    repository,
                    current_session(),
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
        ["run", "inspect", id, options @ ..] => {
            let json = match options {
                [] => false,
                ["--json"] => true,
                _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
            };
            let run = fetch_run(&cli_address(), id)?;
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
        ["run", "stats", "--all"] => print!("{}", stats::render_all(&fetch_runs(ADDRESS)?)),
        ["run", "stats", id] if !id.starts_with('-') => {
            print!("{}", stats::render_run(&fetch_run(ADDRESS, id)?))
        }
        ["run", "requests", "--all", options @ ..] => {
            let (pool, json) = requests_pool::parse_options(options)
                .map_err(|error| format!("{error}\n{HELP}"))?;
            let runs = fetch_runs(ADDRESS)?;
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
            let target = fetch_run(ADDRESS, id)?;
            let runs = fetch_runs(ADDRESS)?;
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
        ["run", "update", id, stage, status, options @ ..] => {
            // The agent's own session is recorded with every update, so its token usage can be attributed.
            let session = current_session();
            let mut options = options.to_vec();
            if let Some(session) = session.as_deref() {
                options.extend(["--session", session]);
            }
            report_run(
                &cli_address(),
                &cli_token_file()?,
                id,
                stage,
                status,
                &options,
            )?;
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
    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String> {
        report_to_dashboard(id, stage, status, options)
    }
}

/// `factory pr open <run-id> --title <title> --body-file <path>`: pushes the run's branch and opens its pull
/// request, or finds the one that is open, and records it on the run.
fn pr_open_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let parsed = pr_open::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    let run = fetch_run(ADDRESS, id)?;
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

/// `factory run check <run-id>`: compares the run's recorded workspace with the repository. It only reads.
/// The exit status is 0 when nothing differs, 2 when something does, and 1 for a problem.
fn run_check_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let json = run_check::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    let run = fetch_run(ADDRESS, id)?;
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

/// `factory run sync <run-id>`: reads the state of the run's pull request from its host and brings the run in
/// line with it: a merged pull request finishes the run, a closed one blocks it, and an open one changes nothing.
fn run_sync_command(id: &str, json: bool) -> Result<u8, Box<dyn std::error::Error>> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    let run = fetch_run(ADDRESS, id)?;
    let Some(url) = run.pr.as_deref() else {
        return Err("The run has no pull request recorded. Record it with `factory run update <id> finish running --pr <url>`.".into());
    };
    let state =
        pr_state::read(&pr_wait::RealGh, url).map_err(|problem| inspect::shown(&problem.0))?;
    let changed = apply_pr_state(id, &run, state)?;
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

/// Applies what the state of the run's pull request means for the run (see `pr_state::follow_up`). Returns
/// whether the run was changed.
fn apply_pr_state(
    id: &str,
    run: &run::Run,
    state: pr_state::PrState,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some(update) = pr_state::follow_up(run, state) else {
        return Ok(false);
    };
    client::send_update(ADDRESS, &cli_token_file()?, id, &update)?;
    Ok(true)
}

/// `factory run cleanup <run-id>`: removes the worktree and local branch of a run whose pull request merged,
/// then clears them from the run. It refuses, removing nothing, when work could be lost.
fn run_cleanup_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    let mut reinstall = false;
    let mut config_dirs: Vec<String> = Vec::new();
    let mut rest = options.iter();
    while let Some(option) = rest.next() {
        match *option {
            "--reinstall" => reinstall = true,
            "--config-dir" => match rest.next() {
                Some(dir) if !dir.starts_with('-') => config_dirs.push((*dir).to_string()),
                _ => return Err(format!("--config-dir needs a path\n{HELP}").into()),
            },
            other => return Err(format!("Unknown option: {other}\n{HELP}").into()),
        }
    }
    let run = fetch_run(ADDRESS, id)?;
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
        let text = |value: serde_json::Value, fallback: &str| {
            value.as_str().unwrap_or(fallback).to_string()
        };
        let stage = text(serde_json::to_value(run.stage)?, "finish");
        let status = text(serde_json::to_value(run.status)?, "running");
        report_run(
            ADDRESS,
            &cli_token_file()?,
            id,
            &stage,
            &status,
            &[
                "--clear-worktree",
                "--clear-branch",
                "--activity",
                "Cleaned up worktree and branch",
            ],
        )?;
    }
    if let (Some(repository), Some(pr)) = (recorded.repository.as_deref(), recorded.pr.as_deref()) {
        let home = install::home_dir()?;
        let var = |name: &str| env::var_os(name);
        let agents = update::installed_agents(&home, &config_dirs, var);
        let claude_dirs = install::claude_config_dirs(&config_dirs, &home, var)?;
        let input = run_reinstall::Inputs {
            repository: std::path::Path::new(repository),
            pr,
            agents: &agents,
            claude_dirs: &claude_dirs,
            tty: std::io::IsTerminal::is_terminal(&std::io::stdin())
                && std::io::IsTerminal::is_terminal(&std::io::stdout()),
            reinstall,
            windows: cfg!(windows),
        };
        run_reinstall::offer(
            &input,
            &pr_wait::RealGh,
            &pr_sync::RealGit,
            &mut run_reinstall::RealIo,
        )
        .map_err(|problem| inspect::shown(&problem.0))?;
    }
    Ok(0)
}

/// `factory pr sync <run-id>`: brings the run's branch up to date with its base. The exit status is the
/// outcome's; 1 is a problem.
fn pr_sync_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let parsed = pr_sync::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    let run = fetch_run(ADDRESS, id)?;
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

/// `factory pr wait <run-id>`: watches the CI of the run's pull request and prints the outcome. The exit
/// status is the outcome's; 1 is a problem with the tools or the run.
fn pr_wait_command(id: &str, options: &[&str]) -> Result<u8, Box<dyn std::error::Error>> {
    let (mut wait_options, json) =
        pr_wait::parse_options(options).map_err(|error| format!("{error}\n{HELP}"))?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id".into());
    }
    let gh = pr_wait::RealGh;
    let run = fetch_run(ADDRESS, id)?;
    if !run.pr.as_deref().is_some_and(pr_wait::is_azure_pr_url) {
        pr_wait::check_gh(&gh).map_err(|problem| problem.0)?;
    }
    // The run's activity follows the wait, keeping its stage and status. Only a running run is updated.
    let stage = serde_json::to_value(run.stage)?
        .as_str()
        .unwrap_or("finish")
        .to_string();
    let running = matches!(run.status, run::RunStatus::Running);
    let mut reported_failure = false;
    let mut activity = |text: &str| {
        if running
            && let Err(error) = report_to_dashboard(id, &stage, "running", &["--activity", text])
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
            Ok(state) => match apply_pr_state(id, &run, state) {
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

/// The real factory server for `factory run begin`.
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
            ADDRESS, &token, title, project, repository, session, routing,
        )
        .map_err(|e| e.to_string())
    }

    fn get(&self, id: &str) -> Result<run_begin::RunSummary, String> {
        let run = fetch_run(ADDRESS, id).map_err(|e| e.to_string())?;
        Ok(run_begin::RunSummary {
            title: run.title,
            finished: matches!(run.status, run::RunStatus::Done),
            worktree: run.worktree,
        })
    }

    fn update(&self, id: &str, stage: &str, status: &str, options: &[&str]) -> Result<(), String> {
        report_to_dashboard(id, stage, status, options)
    }

    fn runs_with_session(&self, session: &str) -> Result<Vec<(String, String)>, String> {
        let runs = fetch_runs(ADDRESS).map_err(|e| e.to_string())?;
        Ok(runs
            .into_iter()
            .filter(|run| run.sessions.iter().any(|known| known == session))
            .map(|run| (run.id, run.title))
            .collect())
    }
}

/// `factory run begin <title>`, or `--run <id>` to resume: creates a run and its workspace.
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
        current_session(),
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

/// The id of the agent session this command runs in, when the agent exposes it (Claude Code does).
fn current_session() -> Option<String> {
    let id = env::var("CLAUDE_CODE_SESSION_ID").ok()?;
    run::check_session_id(&id).ok().map(|()| id)
}

/// `[--port <port>] [--no-usage]` in any order. Usage collection is on unless `--no-usage` or `FACTORY_USAGE=off`.
fn parse_dashboard_options(
    options: &[&str],
    factory_usage: Option<&str>,
) -> Result<(u16, bool), String> {
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
    if usage::switched_off_by(factory_usage) {
        usage = false;
    }
    Ok((port.unwrap_or(DEFAULT_PORT), usage))
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

/// Serves the dashboard, or, when a Factory dashboard already answers on the port, prints its URL.
fn dashboard(port: u16, usage: bool) -> Result<(), Box<dyn std::error::Error>> {
    // A Project is identified by its repository, so a linked worktree opens its main checkout's row.
    let dir = env::current_dir()?;
    let project =
        repository::canonical_repository(&dir).unwrap_or_else(|| dir.display().to_string());
    let url = dashboard_url(port, &project);
    match probe_dashboard(&format!("127.0.0.1:{port}"))? {
        Probe::Running(info) => {
            println!("Dashboard already running (pid {}): {url}", info.pid);
            if !usage {
                eprintln!("Note: --no-usage and FACTORY_USAGE only apply to a dashboard this command starts.");
            }
            let ours = current_build();
            if info.version != ours {
                eprintln!(
                    "Warning: the running dashboard is factory {}, this is factory {ours}. It was left running; stop it and run 'factory dashboard' again to use this version.",
                    info.version
                );
            }
        }
        Probe::Foreign => {
            return Err(format!(
                "Port {port} is in use by something that is not a Factory dashboard (it may be a dashboard from an older Factory; stop it and run 'factory dashboard' again). Or use 'factory dashboard --port <port>'."
            )
            .into())
        }
        Probe::Absent => {
            let home = install::home_dir()?;
            let settings = usage::UsageSettings {
                enabled: usage,
                roots: usage_logs::Roots::from_home(&home, &|name| env::var_os(name))?,
                budget: DASHBOARD_USAGE_BUDGET,
                settle: usage::UsageSettings::default_settle(),
            };
            serve(&data_path(port)?, port, &url, settings)?
        }
    }
    Ok(())
}

/// Reports a run update to the default dashboard, with the CLI token it wrote.
fn report_to_dashboard(
    id: &str,
    stage: &str,
    status: &str,
    options: &[&str],
) -> Result<(), String> {
    let token_file = cli_token_file().map_err(|e| e.to_string())?;
    report_run(ADDRESS, &token_file, id, stage, status, options).map_err(|e| e.to_string())
}

/// The server `run update` talks to. Debug builds honor `FACTORY_TEST_ADDRESS`, so CLI tests can use a
/// fake server on an OS-assigned port; release builds always use the default dashboard.
fn cli_address() -> String {
    #[cfg(debug_assertions)]
    if let Ok(address) = env::var("FACTORY_TEST_ADDRESS") {
        return address;
    }
    ADDRESS.to_string()
}

/// Where the default dashboard writes the CLI token that POSTs must send.
fn cli_token_file() -> io::Result<PathBuf> {
    Ok(server::token_path(&data_path(DEFAULT_PORT)?, DEFAULT_PORT))
}

/// Where the plan of run `id` is saved, in the default dashboard's data dir; its folder is created.
fn plan_path(id: &str) -> io::Result<PathBuf> {
    plans::plan_path_ready(&data_path(DEFAULT_PORT)?, DEFAULT_PORT, id)
}

/// The default dashboard keeps `runs.json`; one on another port keeps its own file and shares nothing.
fn data_path(port: u16) -> io::Result<PathBuf> {
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
    let file = if port == DEFAULT_PORT {
        "runs.json".to_string()
    } else {
        format!("runs-{port}.json")
    };
    Ok(base.join("tds-factory").join(file))
}

#[cfg(test)]
mod test_support {
    pub(crate) struct TestDir(pub(crate) tempfile::TempDir);
    impl TestDir {
        pub(crate) fn new() -> Self {
            Self(tempfile::tempdir().unwrap())
        }
    }

    /// Makes `path` unreadable. Returns false when it can still be read anyway (running as root), so the
    /// caller skips its check instead of failing.
    #[cfg(unix)]
    pub(crate) fn make_unreadable(path: &std::path::Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::read(path).is_err()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_label_shows_the_version_for_releases_and_the_commit_otherwise() {
        assert_eq!(build_label("0.3.1", true, "1a2b3c4"), "0.3.1");
        assert_eq!(build_label("0.3.1", true, ""), "0.3.1");
        assert_eq!(build_label("0.3.1", false, "1a2b3c4"), "dev 1a2b3c4");
        assert_eq!(build_label("0.3.1", false, ""), "dev (unknown commit)");
    }

    #[test]
    fn the_dashboard_url_carries_the_percent_encoded_project() {
        assert_eq!(
            dashboard_url(4311, "/home/me/my repo"),
            "http://127.0.0.1:4311/?project=%2Fhome%2Fme%2Fmy%20repo"
        );
        assert_eq!(
            dashboard_url(4400, "C:\\work\\a&b"),
            "http://127.0.0.1:4400/?project=C%3A%5Cwork%5Ca%26b"
        );
    }

    #[test]
    fn ports_are_numbers_from_1_to_65535() {
        assert_eq!(parse_port("4400"), Ok(4400));
        for bad in ["0", "65536", "-1", "abc", ""] {
            assert!(parse_port(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_separate_port_keeps_its_own_data_file() {
        assert!(
            data_path(DEFAULT_PORT)
                .unwrap()
                .ends_with("tds-factory/runs.json")
        );
        assert!(
            data_path(4400)
                .unwrap()
                .ends_with("tds-factory/runs-4400.json")
        );
    }

    #[test]
    fn dashboard_options_set_the_port_and_the_usage_switch_in_any_order() {
        assert_eq!(parse_dashboard_options(&[], None), Ok((DEFAULT_PORT, true)));
        assert_eq!(
            parse_dashboard_options(&["--no-usage", "--port", "4400"], None),
            Ok((4400, false))
        );
        assert_eq!(
            parse_dashboard_options(&["--port", "4400"], Some("off")),
            Ok((4400, false))
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
