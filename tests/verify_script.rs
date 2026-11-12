//! Behavior of `scripts/verify.sh`, run against a fake `cargo` so no real check executes.
#![cfg_attr(
    unix,
    expect(
        clippy::unwrap_used,
        reason = "integration test helpers fail the test by panicking"
    )
)]
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/verify.sh");

/// Writes an executable `name` into `bin` that logs its arguments to `log`.
fn fake_tool(bin: &Path, log: &Path, name: &str, fail: bool) {
    let script = format!(
        "#!/bin/sh\necho \"{name} $*\" >> '{}'\n{}\n",
        log.display(),
        if fail { "exit 1" } else { "exit 0" }
    );
    let path = bin.join(name);
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Writes a `cargo` into `bin` that logs its arguments and fails when they start with `fail_on`.
fn fake_cargo(bin: &Path, log: &Path, fail_on: &str) {
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in '{fail_on}'*) exit 1 ;; esac\n",
        log.display()
    );
    let path = bin.join("cargo");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Outcome {
    success: bool,
    output: String,
    /// The cargo commands that ran, without the tool availability probes.
    checks: Vec<String>,
}

/// Runs the script with only `bin` on `PATH`; `fail_on` makes the fake cargo fail those commands.
fn verify(with_cargo: bool, fail_on: &str) -> Outcome {
    verify_with(with_cargo, fail_on, &["python3", "chromium"], false)
}

/// Like `verify`, but chooses which of the fake tools exist and whether python3 fails.
fn verify_with(with_cargo: bool, fail_on: &str, tools: &[&str], python_fails: bool) -> Outcome {
    verify_full(with_cargo, fail_on, tools, python_fails, None)
}

/// Like `verify_with`, and `installed_targets` adds a fake `rustup` that lists those targets (none: no rustup at all).
fn verify_full(
    with_cargo: bool,
    fail_on: &str,
    tools: &[&str],
    python_fails: bool,
    installed_targets: Option<&str>,
) -> Outcome {
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("calls.log");
    if with_cargo {
        fake_cargo(bin.path(), &log, fail_on);
    }
    if let Some(targets) = installed_targets {
        let path = bin.path().join("rustup");
        fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{targets}'\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        let grep = ["/usr/bin/grep", "/bin/grep"]
            .into_iter()
            .find(|candidate| Path::new(candidate).exists())
            .unwrap();
        std::os::unix::fs::symlink(grep, bin.path().join("grep")).unwrap();
    }
    for tool in tools {
        fake_tool(bin.path(), &log, tool, *tool == "python3" && python_fails);
    }
    let result = Command::new("/bin/sh")
        .arg(SCRIPT)
        .env("PATH", bin.path())
        .output()
        .unwrap();
    let calls = fs::read_to_string(&log).unwrap_or_default();
    Outcome {
        success: result.status.success(),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
        checks: calls
            .lines()
            .filter(|call| !call.ends_with("--version"))
            .map(str::to_owned)
            .collect(),
    }
}

/// The harness self-test step, then the `python3 tests/dashboard-*.py` steps the script should run: every such file in the repo, in byte order.
fn dashboard_steps() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("dashboard-") && name.ends_with(".py"))
        .collect();
    names.sort();
    assert!(!names.is_empty());
    std::iter::once("python3 -m unittest discover -s tests -p test_*.py".to_owned())
        .chain(
            names
                .into_iter()
                .map(|name| format!("python3 tests/{name}")),
        )
        .collect()
}

#[test]
fn runs_the_ci_checks_in_order_and_reports_success() {
    let run = verify(true, "none");
    assert!(run.success, "{}", run.output);
    assert_eq!(
        run.checks,
        [
            "fmt --check".to_owned(),
            "clippy --all-targets --locked -- -D warnings".to_owned(),
            "test --locked".to_owned(),
        ]
        .into_iter()
        .chain(dashboard_steps())
        .collect::<Vec<_>>()
    );
    assert!(run.output.contains("All checks passed"), "{}", run.output);
}

#[test]
fn stops_at_the_first_failing_check() {
    let run = verify(true, "clippy --all-targets");
    assert!(!run.success);
    assert_eq!(
        run.checks,
        [
            "fmt --check",
            "clippy --all-targets --locked -- -D warnings"
        ]
    );
    assert!(!run.output.contains("All checks passed"));
}
