//! Real git for tests in temporary repositories, built on the hermetic command of `git.rs` and kept away from
//! the user's own settings: no system or global config (so no signing key or hooks) and a fixed author.

use super::{Git, command, exit_code, finish};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    process::Command,
};

/// git in `dir` with `args`, isolated for tests.
fn isolated(dir: &Path, args: &[&str]) -> Command {
    let mut command = command(dir);
    command
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // This file never exists, so the user's global settings stay out.
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("jidoka-tests-no-such-gitconfig"),
        )
        // Local bare remotes stay usable whatever `safe.bareRepository` the machine sets.
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "safe.bareRepository")
        .env("GIT_CONFIG_VALUE_0", "all")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    command
}

/// What isolated git printed, read the same way as the real `Git` reads it.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    finish(isolated(dir, args).output(), args)
}

/// The exit status of isolated git.
pub(crate) fn status(dir: &Path, args: &[&str]) -> Result<i32, String> {
    exit_code(isolated(dir, args).output())
}

/// Runs isolated git that must succeed, for setting up a test, and returns what it printed.
pub(crate) fn run_git(dir: &Path, args: &[&str]) -> String {
    git(dir, args).unwrap_or_else(|error| panic!("git {args:?}: {error}"))
}

/// The plain path form git prints; on Windows `canonicalize` gives a `\\?\` path that git reads as a URL.
pub(crate) fn real_path(path: &Path) -> PathBuf {
    let real = path.canonicalize().unwrap();
    match real.to_string_lossy().strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => real,
    }
}

/// The code's `Git`, isolated like the tests' own. It records every command so a test can see what the
/// code ran.
#[derive(Default)]
pub(crate) struct TestGit {
    pub(crate) commands: RefCell<Vec<Vec<String>>>,
}

impl TestGit {
    fn record(&self, args: &[&str]) {
        self.commands
            .borrow_mut()
            .push(args.iter().map(|arg| arg.to_string()).collect());
    }
}

impl Git for TestGit {
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        self.record(args);
        git(dir, args)
    }

    fn status(&self, dir: &Path, args: &[&str]) -> Result<i32, String> {
        self.record(args);
        status(dir, args)
    }
}
