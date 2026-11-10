//! Behavior of `scripts/install.sh`, run against a fake `curl` and a fake `jidoka` in a temporary home.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#![expect(
    clippy::unwrap_used,
    reason = "integration test helpers fail the test by panicking"
)]

#[expect(
    dead_code,
    reason = "this test uses only `isolate` from the shared helpers"
)]
mod common;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output, Stdio},
};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/install.sh");

/// `setsid` (util-linux) detaches the script from the controlling terminal; without it the tests
/// cannot reproduce a terminal-less run, so they skip instead of running in a different setup.
fn setsid_available() -> bool {
    let found = Command::new("sh")
        .args(["-c", "command -v setsid"])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !found {
        eprintln!(
            "skipped: `setsid` is not installed, so the script cannot run without a terminal"
        );
    }
    found
}

fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Runs the installer with a `curl` that serves a fake `jidoka` recording how `install` was called.
fn run(home: &Path, agent: Option<&str>, dirs: Option<&str>) -> Output {
    let bin = home.join("fakebin");
    fs::create_dir_all(&bin).unwrap();
    let served = home.join("served-jidoka");
    let log = home.join("install.log");
    executable(
        &served,
        &format!(
            "#!/bin/sh\n{{ echo \"args: $*\"; echo \"agent: ${{JIDOKA_AGENT:-}}\"; echo \"dirs: ${{JIDOKA_CLAUDE_CONFIG_DIRS:-}}\"; if [ -t 0 ]; then echo stdin: terminal; else echo stdin: other; fi; }} >> '{}'\n",
            log.display()
        ),
    );
    executable(
        &bin.join("curl"),
        &format!(
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) out=$2; shift ;; *) url=$1 ;; esac; shift; done\ncase \"$url\" in *.sha256) sha256sum '{0}' > \"$out\" ;; *) cp '{0}' \"$out\" ;; esac\n",
            served.display()
        ),
    );
    // Without a controlling terminal `(: </dev/tty)` fails like in CI; `setsid` detaches it.
    let mut command = Command::new("setsid");
    command.args(["--wait", "sh"]);
    command.arg(SCRIPT).env(
        "PATH",
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    );
    common::isolate(&mut command, home);
    command.stdin(Stdio::null());
    if let Some(agent) = agent {
        command.env("JIDOKA_AGENT", agent);
    }
    if let Some(dirs) = dirs {
        command.env("JIDOKA_CLAUDE_CONFIG_DIRS", dirs);
    }
    command.output().unwrap()
}

#[test]
fn installer_runs_the_binary_install_without_prompting_itself() {
    if !setsid_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let output = run(home.path(), None, None);
    assert!(output.status.success(), "{output:?}");
    let log = fs::read_to_string(home.path().join("install.log")).unwrap();
    assert!(log.contains("args: install\n"), "{log}");
    assert!(!log.contains("--agent"), "{log}");
    assert!(!log.contains("stdin: terminal"), "{log}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("No terminal to ask"), "{stderr}");
}

#[test]
fn installer_passes_the_environment_through_to_the_binary() {
    if !setsid_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let output = run(home.path(), Some("codex,claude"), Some("/a b:/c"));
    assert!(output.status.success(), "{output:?}");
    let log = fs::read_to_string(home.path().join("install.log")).unwrap();
    assert!(log.contains("agent: codex,claude\n"), "{log}");
    assert!(log.contains("dirs: /a b:/c\n"), "{log}");
    assert!(log.contains("args: install\n"), "{log}");
}
