mod agent;
mod cli;
mod client;
mod discovery;
mod doctor;
mod document;
mod git;
mod gitignore;
mod group_cli;
mod groups;
mod install;
mod install_record;
mod knowledge;
mod plans;
mod pr;
mod program;
mod repository;
mod reviews;
mod routing;
mod run;
mod server;
mod setup;
mod stages;
#[cfg(test)]
mod test_support;
mod timestamps;
mod token;
mod usage;

use cli::{HELP, execute};
use run::UPDATE_HELP;
use std::{env, io, process::ExitCode};

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

/// Picks the command the arguments name and runs it; returns its exit status, where a non-zero one means it
/// already reported why.
fn run_cli() -> Result<u8, Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Ok(home) = install::home_dir() {
        setup::check(&words, |name| env::var_os(name), &home, &mut io::stderr());
    }
    match words.as_slice() {
        [] | ["--help" | "-h"] => println!("{HELP}\n\n{UPDATE_HELP}"),
        ["--version" | "-V"] => println!("jidoka {}", cli::current_build()),
        ["install", options @ ..] => install::run(options)?,
        ["knowledge", words @ ..] => knowledge::run(words)?,
        ["group", words @ ..] => {
            group_cli::run(words, &client::address()?, &client::token_file()?)?;
        }
        ["doctor", options @ ..] => return execute::<doctor::DoctorCommand>(options),
        ["dashboard", options @ ..] => return execute::<server::DashboardCommand>(options),
        ["run", "start", rest @ ..] if !rest.is_empty() => {
            return execute::<run::begin::StartCommand>(rest);
        }
        ["run", "begin", rest @ ..] if matches!(rest, ["--run", _, ..]) || names(rest) => {
            return execute::<run::begin::BeginCommand>(rest);
        }
        ["run", "cleanup", rest @ ..] if names(rest) => {
            return execute::<run::cleanup::CleanupCommand>(rest);
        }
        ["run", "check", rest @ ..] if names(rest) => {
            return execute::<run::check::CheckCommand>(rest);
        }
        ["run", "sync", rest @ ..] if names(rest) => {
            return execute::<pr::state::RunSyncCommand>(rest);
        }
        ["run", "next", rest @ ..] if names(rest) => {
            return execute::<run::next::NextCommand>(rest);
        }
        ["run", "gate", rest @ ..] if rest.len() >= 3 && names(rest) => {
            return execute::<run::gate::GateCommand>(rest);
        }
        ["run", "advance", rest @ ..] if names(rest) => {
            return execute::<run::gate::AdvanceCommand>(rest);
        }
        ["run", "inspect", rest @ ..] if !rest.is_empty() => {
            return execute::<run::inspect::InspectCommand>(rest);
        }
        ["run", "stats", rest @ ..]
            if matches!(rest, ["--all"]) || (rest.len() == 1 && names(rest)) =>
        {
            return execute::<usage::stats::StatsCommand>(rest);
        }
        ["run", "requests", "--all", options @ ..] => {
            return execute::<usage::requests_pool::PoolCommand>(options);
        }
        ["run", "requests", rest @ ..] if names(rest) => {
            return execute::<usage::requests::RequestsCommand>(rest);
        }
        ["run", "update", rest @ ..] if !rest.is_empty() => {
            return execute::<cli::UpdateCommand>(rest);
        }
        ["run", rest @ ..] if matches!(rest, ["block" | "resume" | "rework", _, ..]) => {
            return execute::<cli::LifecycleCommand>(rest);
        }
        ["pr", "wait", rest @ ..] if names(rest) => return execute::<pr::wait::WaitCommand>(rest),
        ["pr", "open", rest @ ..] if names(rest) => return execute::<pr::open::OpenCommand>(rest),
        ["pr", "sync", rest @ ..] if names(rest) => return execute::<pr::sync::SyncCommand>(rest),
        _ => return Err(cli::usage_error("Invalid arguments.").into()),
    }
    Ok(0)
}

/// Whether a command's words start with what it works on, such as a run id, rather than an option.
fn names(rest: &[&str]) -> bool {
    rest.first().is_some_and(|word| !word.starts_with('-'))
}
