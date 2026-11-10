//! Shared by the integration tests that run the `jidoka` binary.

use std::{path::Path, process::Command};

/// The `jidoka` binary with every home, config and data location pointing inside `home`.
///
/// The startup check adopts installs and refreshes skills, so a test must never reach the real home
/// of whoever runs it. A test that needs a different value overrides it on the returned command.
pub fn jidoka_in(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jidoka"));
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
        .env("COPILOT_HOME", home.join(".copilot"));
    command
}
