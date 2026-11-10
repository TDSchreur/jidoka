//! After `factory run cleanup`, offers to reinstall the factory when the Run changed the factory itself: the
//! repository's own build replaces the installed command and skills, so the next Run uses what just merged.

use crate::{
    inspect::shown,
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

    /// Whether this command cannot be printed as a paste-ready line on Windows.
    fn unsafe_on_windows(&self) -> bool {
        windows_unsafe(&self.program)
            || self
                .args
                .iter()
                .any(|a| windows_unsafe(&a.to_string_lossy()))
    }

    /// The command as a shell line that can be pasted, and is safe to print on a terminal.
    pub(crate) fn display(&self, windows: bool) -> String {
        let args = self.args.iter().map(|a| a.to_string_lossy());
        let line = std::iter::once(self.program.as_str().into())
            .chain(args)
            .map(|word| quote(&word, windows))
            .collect::<Vec<_>>()
            .join(" ");
        shown(&line)
    }
}

/// Whether `word` can be pasted into both PowerShell and cmd once double-quoted: only ASCII letters and
/// digits, spaces and `_ . - : \ / ( ) ~`. `~` is allowed because Windows temp and user folders are often
/// 8.3 short names such as `RUNNER~1`. Anything else (expansion, escapes, separators, look-alike quotes)
/// could be interpreted by one of them.
fn windows_safe(word: &str) -> bool {
    word.chars()
        .all(|c| c.is_ascii_alphanumeric() || " _.-:\\/()~".contains(c))
}

/// Whether `word` cannot be quoted safely for both PowerShell and cmd.
fn windows_unsafe(word: &str) -> bool {
    !windows_safe(word)
}

/// `word` as one shell word: left alone when it is plain, otherwise in POSIX single quotes, or in double
/// quotes on Windows. On Windows `~` is not plain, so a word with it (such as `RUNNER~1`) is double-quoted,
/// where it is inert in PowerShell and cmd. Double quotes only keep spaces together there: PowerShell still
/// expands `$` and the backtick and cmd `%`, and `\"` means nothing to cmd, so a Windows word with such
/// characters must not be pasted at all (see `windows_safe`).
pub(crate) fn quote(word: &str, windows: bool) -> String {
    let plain = |c: char| {
        c.is_ascii_alphanumeric() || (if windows { "_-./:\\" } else { "_-./:=,@+" }).contains(c)
    };
    if !word.is_empty() && word.chars().all(plain) {
        word.to_string()
    } else if windows {
        format!("\"{}\"", word.replace('"', "\\\""))
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
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
        println!("$ {}", command.display(false));
        let status = std::process::Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.dir)
            .status()
            .map_err(|error| shown(&error.to_string()))?;
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
    /// Whether this is Windows, where the running `factory.exe` is locked and cannot replace itself.
    pub(crate) windows: bool,
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
        } else if in_package && let Some((key, value)) = line.split_once('=') {
            named |= key.trim() == "name" && value.trim().trim_matches('"') == "factory";
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

fn listed(commands: &[Command], windows: bool) -> String {
    commands
        .iter()
        .map(|c| {
            if windows && c.unsafe_on_windows() {
                by_hand(&c.dir)
            } else {
                format!("  {}\n", c.display(windows))
            }
        })
        .collect()
}

/// A line that sends the user to type the command themselves, because the path has characters a Windows shell
/// would interpret. The path is shown escaped for reading and is not part of a command.
fn by_hand(repository: &Path) -> String {
    format!(
        "  Run cargo install and factory install by hand for this repository (its path has characters a Windows shell would interpret, so no paste-ready command is printed). Path, not a command: {}\n",
        shown(&repository.display().to_string())
    )
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
            "The merged change is not in the checkout of {} yet, or its merge commit could not be read. Pull the base branch there first, then reinstall the factory:\n  {}\n{}",
            shown(&input.repository.display().to_string()),
            if input.windows && windows_unsafe(&input.repository.to_string_lossy()) {
                "Pull the base branch there by hand (see the path above).".to_string()
            } else {
                shown(&format!(
                    "git -C {} pull",
                    quote(&input.repository.to_string_lossy(), input.windows)
                ))
            },
            listed(&commands, input.windows)
        ));
        return Ok(());
    }
    if input.windows {
        io.say(&format!(
            "The running factory.exe is locked on Windows, so run these after this command exits:\n{}",
            listed(&commands, input.windows)
        ));
        return Ok(());
    }
    let agents = input.agents.join(", ");
    if !input.reinstall {
        if !input.tty {
            io.say(&format!(
                "This run changed the factory itself. To reinstall it for {agents}, run:\n{}",
                listed(&commands, input.windows)
            ));
            return Ok(());
        }
        io.say(&format!(
            "This run changed the factory itself. Reinstalling runs:\n{}",
            listed(&commands, input.windows)
        ));
        let from = shown(&input.repository.display().to_string());
        if !io.confirm(&format!(
            "Reinstall the factory for {agents} from {from}? [y/N]"
        )) {
            return Ok(());
        }
    }
    for command in &commands {
        io.run(command).map_err(|error| {
            ToolProblem(format!(
                "`{}` failed: {}. The cleanup itself succeeded.",
                command.display(input.windows),
                shown(&error)
            ))
        })?;
    }
    io.say("Reinstalled the factory. Restart the dashboard to use it: factory dashboard\n");
    Ok(())
}

#[cfg(test)]
#[path = "run_reinstall_tests.rs"]
mod tests;
