mod client;
mod doctor;
mod inspect;
mod install;
mod repository;
mod routing;
mod run;
mod server;
mod stats;
mod update;
mod usage;
mod usage_logs;

use client::{fetch_run, fetch_runs, probe_dashboard, report_run, start_run, Probe};
use run::UPDATE_HELP;
use server::{serve, ADDRESS, DEFAULT_PORT};
use std::{env, io, path::PathBuf, process::ExitCode};

const HELP: &str = "TDS Factory\n\nUsage:\n  factory install --agent <codex|claude|copilot>[,...]\n  factory dashboard [--port <port>] [--no-usage]\n  factory run stats <id> | --all\n  factory check\n  factory update [--agent <codex|claude|copilot>[,...]]\n  factory doctor [--agent <codex|claude|copilot>] [--json] [--create-model-routing]\n  factory run start <title> [--agent <codex|claude|copilot>]\n  factory run inspect <id> [--json]\n  factory run update <id> <define|build|check|finish> <running|blocked|done> [options]\n  factory --help | --version";

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
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the command; `false` means it already reported why it did not succeed.
fn run_cli() -> Result<bool, Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--help" | "-h"] => println!("{HELP}\n\n{UPDATE_HELP}"),
        ["--version" | "-V"] => println!("factory {}", current_build()),
        ["install", "--agent", agent] => install::install(agent)?,
        ["update", options @ ..] => {
            let agents = match options {
                [] => None,
                ["--agent", agents] => Some(*agents),
                _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
            };
            update::update(
                env!("CARGO_PKG_VERSION"),
                env!("FACTORY_BUILD_RELEASE") == "true",
                agents,
            )?
        }
        ["check"] => update::check(
            env!("CARGO_PKG_VERSION"),
            env!("FACTORY_BUILD_RELEASE") == "true",
        )?,
        ["doctor", options @ ..] => {
            let args = doctor::parse_args(options).map_err(|error| format!("{error}\n{HELP}"))?;
            return doctor::run(args);
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
            // Read once, before the run exists: a mistake in the policy, or a policy for several agents
            // without `--agent`, stops the start.
            let resolution = routing::policy_path_in(|name| env::var_os(name)).map_or_else(
                || Ok(routing::Resolution::default()),
                |path| routing::load(&path, host),
            )?;
            if let Some(notice) = &resolution.notice {
                eprintln!("{notice}");
            }
            println!(
                "{}",
                start_run(
                    ADDRESS,
                    title,
                    &dir.display().to_string(),
                    repository,
                    current_session(),
                    resolution.selection
                )?
            );
        }
        ["run", "inspect", id, options @ ..] => {
            let json = match options {
                [] => false,
                ["--json"] => true,
                _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
            };
            let run = fetch_run(ADDRESS, id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&run)?);
            } else {
                print!("{}", inspect::render(&run));
            }
        }
        ["run", "stats", "--all"] => print!("{}", stats::render_all(&fetch_runs(ADDRESS)?)),
        ["run", "stats", id] if !id.starts_with('-') => {
            print!("{}", stats::render_run(&fetch_run(ADDRESS, id)?))
        }
        ["run", "update", id, stage, status, options @ ..] => {
            // The agent's own session is recorded with every update, so its token usage can be attributed.
            let session = current_session();
            let mut options = options.to_vec();
            if let Some(session) = session.as_deref() {
                options.extend(["--session", session]);
            }
            report_run(ADDRESS, id, stage, status, &options)?;
            println!("Updated run {id}");
        }
        _ => return Err(format!("Invalid arguments.\n{HELP}").into()),
    }
    Ok(true)
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
                budget: std::time::Duration::from_secs(5),
                settle: usage::UsageSettings::default_settle(),
            };
            serve(&data_path(port)?, port, &url, settings)?
        }
    }
    Ok(())
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
        assert!(data_path(DEFAULT_PORT)
            .unwrap()
            .ends_with("tds-factory/runs.json"));
        assert!(data_path(4400)
            .unwrap()
            .ends_with("tds-factory/runs-4400.json"));
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
