//! After `factory run cleanup`, offers to reinstall the factory when the Run changed the factory itself: the
//! repository's own build replaces the installed command and skills, so the next Run uses what just merged.

use crate::{
    pr_state,
    pr_sync::Git,
    pr_wait::{Gh, ToolProblem},
    update,
};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

/// A command to run in `dir`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Command {
    pub(crate) program: String,
    pub(crate) args: Vec<OsString>,
    pub(crate) dir: PathBuf,
}

impl Command {
    fn new(program: &str, args: Vec<OsString>, dir: &Path) -> Self {
        Command {
            program: program.into(),
            args,
            dir: dir.to_path_buf(),
        }
    }

    pub(crate) fn display(&self) -> String {
        let args = self.args.iter().map(|a| a.to_string_lossy());
        std::iter::once(self.program.as_str().into())
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Everything that touches the outside world, so the decision can be tested.
pub(crate) trait Io {
    fn say(&mut self, text: &str);
    /// Asks a yes/no question, default no.
    fn confirm(&mut self, question: &str) -> bool;
    fn run(&mut self, command: &Command) -> Result<(), String>;
}

/// The terminal and the child processes.
pub(crate) struct RealIo;

impl Io for RealIo {
    fn say(&mut self, text: &str) {
        print!("{text}");
    }

    fn confirm(&mut self, question: &str) -> bool {
        print!("{question} ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).is_ok()
            && matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
    }

    fn run(&mut self, command: &Command) -> Result<(), String> {
        println!("$ {}", command.display());
        let status = std::process::Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.dir)
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(status.to_string())
        }
    }
}

pub(crate) struct Inputs<'a> {
    pub(crate) repository: &'a Path,
    pub(crate) pr: &'a str,
    /// The agents with the factory installed.
    pub(crate) agents: &'a [&'static str],
    pub(crate) claude_dirs: &'a [PathBuf],
    /// Whether stdin and stdout are a terminal, so that a question can be asked.
    pub(crate) tty: bool,
    pub(crate) reinstall: bool,
}

/// Whether `repository` is the factory's own source: its crate is `factory` and it ships the procedures skill.
pub(crate) fn is_factory(repository: &Path) -> bool {
    let Ok(manifest) = fs::read_to_string(repository.join("Cargo.toml")) else {
        return false;
    };
    let mut in_package = false;
    let mut named = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            if let Some((key, value)) = line.split_once('=') {
                named |= key.trim() == "name" && value.trim().trim_matches('"') == "factory";
            }
        }
    }
    named
        && repository
            .join(".agents/skills/factory-procedures/SKILL.md")
            .is_file()
}

fn commands(input: &Inputs) -> [Command; 2] {
    let agents = input.agents.join(",");
    let dirs: &[PathBuf] = if input.agents.contains(&"claude") {
        input.claude_dirs
    } else {
        &[]
    };
    [
        Command::new(
            "cargo",
            ["install", "--path"]
                .map(OsString::from)
                .into_iter()
                .chain([input.repository.into(), "--locked".into(), "--force".into()])
                .collect(),
            input.repository,
        ),
        // The freshly installed command, found on PATH like every other use of `factory`.
        Command::new(
            "factory",
            update::install_args(&agents, dirs),
            input.repository,
        ),
    ]
}

fn listed(commands: &[Command]) -> String {
    commands
        .iter()
        .map(|c| format!("  {}\n", c.display()))
        .collect()
}

/// Offers (or, with `reinstall`, performs) the reinstall. Nothing happens unless the repository is the factory
/// itself. An error means a reinstall command failed; the cleanup had already succeeded.
pub(crate) fn offer(
    input: &Inputs,
    gh: &dyn Gh,
    git: &dyn Git,
    io: &mut dyn Io,
) -> Result<(), ToolProblem> {
    if !is_factory(input.repository) {
        return Ok(());
    }
    // Also after a cleanup that found nothing left to remove, as long as the pull request is merged.
    if pr_state::read(gh, input.pr).ok() != Some(pr_state::PrState::Merged) {
        return Ok(());
    }
    if input.agents.is_empty() {
        io.say("The factory is not installed for any agent, so there is nothing to reinstall.\n");
        return Ok(());
    }
    let commands = commands(input);
    let contained = pr_state::merge_commit(gh, input.pr)
        .ok()
        .is_some_and(|merge| {
            git.status(
                input.repository,
                &["merge-base", "--is-ancestor", &merge, "HEAD"],
            ) == Ok(0)
        });
    if !contained {
        io.say(&format!(
            "The merged change is not in the checkout of {} yet, or its merge commit could not be read. Pull the base branch there first, then reinstall the factory:\n  git -C {} pull\n{}",
            input.repository.display(),
            input.repository.display(),
            listed(&commands)
        ));
        return Ok(());
    }
    let agents = input.agents.join(", ");
    if !input.reinstall {
        if !input.tty {
            io.say(&format!(
                "This run changed the factory itself. To reinstall it for {agents}, run:\n{}",
                listed(&commands)
            ));
            return Ok(());
        }
        if !io.confirm(&format!("Reinstall the factory for {agents}? [y/N]")) {
            return Ok(());
        }
    }
    for command in &commands {
        io.run(command).map_err(|error| {
            ToolProblem(format!(
                "`{}` failed: {error}. The cleanup itself succeeded.",
                command.display()
            ))
        })?;
    }
    io.say("Reinstalled the factory. Restart the dashboard to use it: factory dashboard\n");
    Ok(())
}

#[cfg(test)]
#[path = "run_reinstall_tests.rs"]
mod tests;
