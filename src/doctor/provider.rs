//! The provider section of `factory doctor`: which pull request host origin belongs to, whether that
//! host's command line tool is installed, and whether it is signed in, as far as a read-only command shows.
//! None of these checks blocks readiness: a missing tool or sign-in is a warning.

use super::{find_program, git_output, Check, Machine};
use crate::pr_open::{detect_host, Host};
use std::{
    path::Path,
    process::{Command, Stdio},
};

/// The environment variable the azure-devops extension reads a personal access token from.
const AZURE_PAT: &str = "AZURE_DEVOPS_EXT_PAT";

pub(super) fn checks(project: &Path, machine: &Machine) -> Vec<Check> {
    let host = match detect_host(&|args| git_output(project, args)) {
        Ok(origin) => origin.host,
        // Detection asks for origin first, so a failure here with no origin means there is none.
        Err(_) if git_output(project, &["remote", "get-url", "origin"]).is_none() => {
            return vec![Check::skipped(
                "host",
                "there is no `origin` remote, so the pull request host was not checked",
            )];
        }
        Err(problem) => {
            return vec![Check::warning(
                "host",
                format!("the pull request host is not supported as configured: {}", problem.0),
                "Fix origin or the factory.prProvider git config, or open pull requests with the host's own tooling and record them with `factory run update <id> <stage> <status> --pr <url>`.",
            )]
        }
    };
    let (host_check, tool_checks) = match host {
        Host::GitHub => (
            Check::pass("host", "pull requests go to GitHub"),
            github_checks(machine),
        ),
        Host::Azure(repo) => (
            Check::pass(
                "host",
                format!(
                    "pull requests go to Azure Repos ({}/{}/{})",
                    repo.organization, repo.project, repo.repository
                ),
            ),
            azure_checks(machine),
        ),
        Host::Unknown => {
            return vec![Check::warning(
                "host",
                "origin is not on a host the factory opens pull requests on (GitHub or Azure Repos)",
                "Open pull requests and watch CI with the host's own tooling, and record the pull request with `factory run update <id> <stage> <status> --pr <url>`. For GitHub Enterprise, run `git config factory.prProvider github`.",
            )]
        }
    };
    let mut checks = vec![host_check];
    checks.extend(tool_checks);
    checks.push(Check::skipped(
        "not-verified",
        "not checked: whether the token is valid, project permissions, and branch policies",
    ));
    checks
}

fn github_checks(machine: &Machine) -> Vec<Check> {
    let Some(gh) = find_program("gh", &machine.path) else {
        return vec![Check::warning(
            "gh",
            "`gh` is not on PATH",
            "Install the GitHub CLI and make sure `gh` is on PATH.",
        )];
    };
    let sign_in = if succeeds(&gh, &["auth", "status"]) {
        Check::pass("sign-in", "`gh auth status` reports a signed-in account")
    } else {
        Check::warning(
            "sign-in",
            "`gh auth status` reports no signed-in account",
            "Run `gh auth login`.",
        )
    };
    vec![
        Check::pass("gh", format!("`gh` found at {}", gh.display())),
        sign_in,
    ]
}

fn azure_checks(machine: &Machine) -> Vec<Check> {
    let Some(az) = find_program("az", &machine.path) else {
        return vec![Check::warning(
            "az",
            "`az` is not on PATH",
            "Install the Azure CLI and its azure-devops extension (`az extension add --name azure-devops`).",
        )];
    };
    let extension = if succeeds(&az, &["extension", "show", "--name", "azure-devops"]) {
        Check::pass(
            "azure-devops-extension",
            "the azure-devops extension is installed",
        )
    } else {
        Check::warning(
            "azure-devops-extension",
            "the azure-devops extension of `az` is not installed",
            "Run `az extension add --name azure-devops`.",
        )
    };
    // Only whether the token is set: it is never read further, validated or printed.
    let pat = (machine.var)(AZURE_PAT).is_some_and(|value| !value.is_empty());
    let sign_in = if pat {
        Check::pass("sign-in", format!("{AZURE_PAT} is set"))
    } else if succeeds(&az, &["account", "show"]) {
        Check::pass("sign-in", "`az account show` reports a signed-in account")
    } else {
        Check::warning(
            "sign-in",
            format!("`az account show` reports no signed-in account and {AZURE_PAT} is not set"),
            format!("Run `az login`, or set {AZURE_PAT} to a personal access token."),
        )
    };
    vec![
        Check::pass("az", format!("`az` found at {}", az.display())),
        extension,
        sign_in,
    ]
}

/// Whether `program args` exits successfully. Its output is discarded, so nothing it prints, such as
/// account details, reaches the report; it cannot prompt because standard input is closed.
fn succeeds(program: &Path, args: &[&str]) -> bool {
    // A relative path, such as one found through an empty `PATH` entry, would run a file from the project.
    program.is_absolute()
        && Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("GH_PROMPT_DISABLED", "1")
            .status()
            .is_ok_and(|status| status.success())
}
