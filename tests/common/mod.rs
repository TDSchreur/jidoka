//! Shared by the integration tests that run the `jidoka` binary.

use std::{path::Path, process::Command};

/// Points every home, config and data location of `command` inside `home` and clears the variables
/// that choose agents, so a test never reaches the real home of whoever runs it.
pub fn isolate(command: &mut Command, home: &Path) {
    command
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("APPDATA", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("LOCALAPPDATA", home.join("data"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("COPILOT_HOME", home.join(".copilot"))
        .env_remove("JIDOKA_AGENT")
        .env_remove("JIDOKA_CLAUDE_CONFIG_DIRS");
}

/// The `jidoka` binary isolated inside `home`.
///
/// The startup check adopts installs and refreshes skills, so a test must never reach the real home
/// of whoever runs it. A test that needs a different value, such as `JIDOKA_AGENT`, sets it on the
/// returned command.
pub fn jidoka_in(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jidoka"));
    isolate(&mut command, home);
    command
}
