//! The one shape every `jidoka` command has: parse, run, render.
//!
//! `main` only picks the command; each command lives in the module that owns its work, and `execute` prints
//! what it renders and returns its exit status.

use crate::{client, run};
use serde::Serialize;
use std::{error::Error, fmt::Display};

pub(crate) const HELP: &str = "Jidoka\n\nUsage:\n  jidoka install [--agent <codex|claude|copilot>[,...]]   (asks which agents when none are given on a terminal; else JIDOKA_AGENT)\n  jidoka dashboard [--port <port>] [--no-usage]\n  jidoka run stats <id> | --all\n  jidoka pr open <run-id> --title <title> --body-file <path> [--base <branch>] [--json]\n  jidoka pr sync <run-id> [--base <branch>] [--json]\n  jidoka pr wait <run-id> [--timeout <duration>] [--fail-fast] [--json]\n  jidoka run requests <id> | --all [--include-partial] [--exclude <id>]... [--json]\n  jidoka doctor [--agent <codex|claude|copilot>] [--json] [--create-model-routing]\n  jidoka run start <title> [--agent <codex|claude|copilot>]\n  jidoka run begin <title> | --run <id> [--agent <codex|claude|copilot>] [--mode <full|balanced|fast|jidoka-full|jidoka-balanced|jidoka-fast>] [--base <branch>] [--branch <name>] [--json]   (also prints the Define stage's `run next`; --json adds it as `next`)\n  jidoka run inspect <id> [--json]   (shows the change group the run is in, if any)\n  jidoka run check <run-id> [--json]\n  jidoka run next <id> [--json]   (current stage, instructions, open gates, allowed moves, summaries, tickets and routed model)\n  jidoka run gate <id> <gate> <pass|fail> [--check <name>] [--evidence <text>]\n  jidoka run advance <id> [--summary <stage>=<text>] [--activity <text>] [--json] [--force --reason <text> | --skip-ci --reason <text>]   (saves the summary and activity with the transition, then prints the new stage's `run next`; --force skips the gates, still follows the stage graph and records the reason; --skip-ci skips only the CI check in Finish)\n  jidoka run sync <run-id> [--json]\n  jidoka run cleanup <run-id> [--discard [--force]]   (--discard also removes a run whose pull request did not merge; --force loses uncommitted or unpushed work)\n  jidoka run update <id> [options]   (reports metadata without changing status)\n  jidoka run block <id> --reason <text>\n  jidoka run resume <id> --source <text>\n  jidoka run rework <id> --finding <id>=<wording|behavior> ... [--decision <text>]   (one --finding for each open Finding)\n  jidoka knowledge [show]\n  jidoka knowledge set <path>   (the knowledge repo checkout; per user)\n  jidoka knowledge checkout <name> <path>   (a repository's checkout on this machine)\n  jidoka group create <title> --spec <url|path> [--run <id>]...   (change groups link runs to one shared spec; a file of the knowledge repo becomes its web link)\n  jidoka group list [--json]\n  jidoka group inspect <group-id> [--json]\n  jidoka group add <group-id> <run-id>\n  jidoka group remove <group-id> <run-id>\n  jidoka group delete <group-id>   (only a group with no runs)\n  jidoka group milestone add <group-id> <text>\n  jidoka group milestone remove <group-id> <milestone-id>\n  jidoka group depend <group-id> <run-id> --on-pr <run-id> | --milestone <milestone-id>\n  jidoka group undepend <group-id> <dependency-id>\n  jidoka group confirm <group-id> <milestone-id> [--evidence <url>]\n  jidoka group reopen <group-id> <milestone-id>   (dependencies are advisory: they are shown, never enforced)\n  jidoka --help | --version\n\nSetup:\n  `jidoka install` writes an install record (agents and skills version) next to the model routing policy. A command run before Setup prints a hint to run `jidoka install`; after an upgrade, the first command refreshes the installed skills automatically.";

/// `error` followed by the usage text, for a command line that cannot be read.
pub(crate) fn usage_error(error: impl Display) -> String {
    format!("{error}\n{HELP}")
}

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

pub(crate) fn current_build() -> String {
    build_label(
        env!("CARGO_PKG_VERSION"),
        env!("JIDOKA_BUILD_RELEASE") == "true",
        env!("JIDOKA_BUILD_COMMIT"),
    )
}

/// One `jidoka` command. `execute` runs every command the same way: parse, run, render, print.
pub(crate) trait Command {
    /// What the command line asked for.
    type Options;
    /// What running the command produced, for `render`.
    type Outcome;

    /// Reads the command's words: everything after its name.
    ///
    /// A command that works on a run refuses a bad id with `client::check_run_id`, here or when it first reads
    /// the run, before anything looks for the dashboard.
    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn Error>>;

    /// Does the command's work. Progress may go to stderr while it runs.
    fn run(options: &Self::Options) -> Result<Self::Outcome, Box<dyn Error>>;

    /// What the command prints and its exit status.
    fn render(options: &Self::Options, outcome: Self::Outcome) -> Result<Output, Box<dyn Error>>;
}

/// What a command prints, stdout first, and its exit status.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Output {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit_code: u8,
}

impl Output {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            stdout: text.into(),
            ..Self::default()
        }
    }

    /// `value` as pretty JSON on its own line.
    pub(crate) fn json(value: &impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self::text(format!(
            "{}\n",
            serde_json::to_string_pretty(value)?
        )))
    }

    /// `value` as JSON when `json` is set, else the text `text` makes.
    pub(crate) fn json_or_text(
        json: bool,
        value: &impl Serialize,
        text: impl FnOnce() -> String,
    ) -> Result<Self, serde_json::Error> {
        if json {
            Self::json(value)
        } else {
            Ok(Self::text(text()))
        }
    }

    pub(crate) fn with_exit_code(self, exit_code: u8) -> Self {
        Self { exit_code, ..self }
    }

    /// This output, then `error` as a failure: what a command prints when it fails after it did its work.
    pub(crate) fn failed(mut self, error: impl Display) -> Self {
        self.stderr.push_str(&format!("{error}\n"));
        self.exit_code = 1;
        self
    }
}

/// Runs command `C` with its `words` and prints its output; returns its exit status.
pub(crate) fn execute<C: Command>(words: &[&str]) -> Result<u8, Box<dyn Error>> {
    let options = C::parse(words)?;
    let outcome = C::run(&options)?;
    let output = C::render(&options, outcome)?;
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    Ok(output.exit_code)
}

/// A command's first word, such as a run id or a title, and the options after it.
///
/// The dispatcher only picks such a command when its first word is there; without one it is empty.
pub(crate) fn split_first<'a, 'b>(words: &'a [&'b str]) -> (&'b str, &'a [&'b str]) {
    match words {
        [id, options @ ..] => (id, options),
        [] => ("", &[]),
    }
}

/// `--json` or nothing, the options of the commands that have no others.
pub(crate) fn parse_json_flag(options: &[&str]) -> Result<bool, String> {
    match options {
        [] => Ok(false),
        ["--json"] => Ok(true),
        _ => Err(usage_error("Invalid arguments.")),
    }
}

/// `jidoka run update <id> [options]`: reports metadata to the run, with the agent sessions this command runs in.
pub(crate) struct UpdateCommand;

impl Command for UpdateCommand {
    type Options = (String, Vec<String>);
    type Outcome = ();

    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn Error>> {
        let (id, options) = split_first(words);
        Ok((
            id.to_owned(),
            options.iter().map(|&o| o.to_owned()).collect(),
        ))
    }

    fn run((id, options): &Self::Options) -> Result<(), Box<dyn Error>> {
        let options = run::begin::with_sessions(options);
        let options: Vec<&str> = options.iter().map(String::as_str).collect();
        Ok(client::report(id, &options)?)
    }

    fn render((id, _): &Self::Options, (): ()) -> Result<Output, Box<dyn Error>> {
        Ok(Output::text(format!("Updated run {id}\n")))
    }
}

/// `jidoka run block|resume|rework <id> ...`: changes the run's lifecycle; `rework` then shows the new stage.
pub(crate) struct LifecycleCommand;

impl Command for LifecycleCommand {
    type Options = (String, run::LifecycleRequest, bool);
    type Outcome = Option<run::next::Next>;

    fn parse(words: &[&str]) -> Result<Self::Options, Box<dyn Error>> {
        let [action, id, options @ ..] = words else {
            return Err(usage_error("Invalid arguments.").into());
        };
        let request = run::parse_lifecycle(action, options)?;
        Ok(((*id).to_owned(), request, *action == "rework"))
    }

    fn run((id, request, rework): &Self::Options) -> Result<Self::Outcome, Box<dyn Error>> {
        client::send_lifecycle(id, request)?;
        Ok(if *rework {
            Some(run::next::next_for(id)?)
        } else {
            None
        })
    }

    fn render((id, _, _): &Self::Options, next: Self::Outcome) -> Result<Output, Box<dyn Error>> {
        Ok(Output::text(next.map_or_else(
            || format!("Updated run {id}\n"),
            |next| run::next::render(&next),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Agent;

    #[test]
    fn help_lists_every_agent_wherever_it_names_one() {
        let expected = Agent::ALL.map(Agent::name).join("|");
        let mut groups = 0;
        for group in HELP.split('<').skip(1) {
            let group = group.split('>').next().unwrap();
            if Agent::ALL.iter().any(|agent| group.contains(agent.name())) {
                assert_eq!(group, expected);
                groups += 1;
            }
        }
        assert!(groups > 0);
    }
}
