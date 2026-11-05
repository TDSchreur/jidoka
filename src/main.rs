mod client;
mod install;
mod run;
mod server;

use client::{report_run, start_run};
use run::UPDATE_HELP;
use server::{serve, ADDRESS};
use std::{env, io, path::PathBuf, process::ExitCode};

const HELP: &str = "TDS Factory\n\nUsage:\n  factory install --agent <codex|claude|copilot>[,...]\n  factory dashboard\n  factory run start <title>\n  factory run update <id> <define|build|check|finish> <running|blocked|done> [options]\n  factory --help | --version";

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
        ["--version" | "-V"] => println!("factory {}", current_build()),
        ["install", "--agent", agent] => install::install(agent)?,
        ["dashboard"] => serve(&data_path()?)?,
        ["run", "start", title] => {
            let project = env::current_dir()?.display().to_string();
            println!("{}", start_run(ADDRESS, title, &project)?);
        }
        ["run", "update", id, stage, status, options @ ..] => {
            report_run(ADDRESS, id, stage, status, options)?;
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
    Ok(base.join("tds-factory/runs.json"))
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
}
