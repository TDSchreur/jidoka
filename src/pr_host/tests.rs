//! The shared adapter suite: the same cases run against the GitHub and the Azure Repos adapter, through
//! `Hosts`, with a scripted `gh` and `az`. Only these tests script the hosts' tools.

use super::*;
use serde_json::json;
use std::cell::RefCell;

/// Answers every call with one scripted answer and records the calls, as `gh <args>` or `az <args>`.
struct Script {
    answer: Result<ToolOutput, String>,
    calls: RefCell<Vec<String>>,
}

impl Script {
    fn printing(stdout: String) -> Self {
        Self::answering(Ok(ToolOutput {
            code: Some(0),
            stdout,
            stderr: String::new(),
        }))
    }

    fn answering(answer: Result<ToolOutput, String>) -> Self {
        Self {
            answer,
            calls: RefCell::new(Vec::new()),
        }
    }

    /// A tool that is not installed.
    fn missing() -> Self {
        Self::answering(Err(String::new()))
    }

    fn reply(&self, call: String) -> Result<ToolOutput, ToolError> {
        self.calls.borrow_mut().push(call);
        self.answer.clone().map_err(|_| ToolError::NotInstalled)
    }
}

impl Tools for Script {
    fn gh(&self, _dir: Option<&Path>, args: &[&str]) -> Result<ToolOutput, ToolError> {
        self.reply(format!("gh {}", args.join(" ")))
    }

    fn az(&self, args: &[&str]) -> Result<ToolOutput, ToolError> {
        self.reply(format!("az {}", args.join(" ")))
    }
}

/// One host as the shared cases see it.
struct Case {
    url: &'static str,
    /// The one call that reads the state.
    state_call: &'static str,
    /// What the host's tool prints for a pull request in `state` from branch `feature/x` at commit
    /// `ABCD1234`; `PrState::Unknown` stands for a state the host reports that Jidoka does not know.
    print: fn(PrState) -> String,
    /// What the tool prints to stderr, and its exit status, when it is not signed in.
    signed_out: (i32, &'static str),
    signed_out_message: &'static str,
    not_installed_message: &'static str,
}

fn github() -> Case {
    Case {
        url: "https://github.com/o/r/pull/7",
        state_call: "gh pr view https://github.com/o/r/pull/7 --json state,isDraft",
        print: |state| {
            let (state, draft) = match state {
                PrState::Open => ("OPEN", false),
                PrState::Draft => ("OPEN", true),
                PrState::Merged => ("MERGED", false),
                PrState::Closed => ("CLOSED", false),
                PrState::Unknown => ("WEIRD", false),
            };
            json!({"state": state, "isDraft": draft, "headRefOid": "ABCD1234", "headRefName": "feature/x"})
                .to_string()
        },
        signed_out: (
            4,
            "To get started with GitHub CLI, please run:  gh auth login",
        ),
        signed_out_message: "The GitHub CLI is not signed in. Run `gh auth login` and try again.",
        not_installed_message: "The GitHub CLI (`gh`) is not installed or not on PATH. Install it from https://cli.github.com, run `gh auth login`, and try again.",
    }
}

fn azure() -> Case {
    Case {
        url: "https://dev.azure.com/org/Proj/_git/Repo/pullrequest/535",
        state_call: "az repos pr show --id 535 --organization https://dev.azure.com/org --output json --detect false",
        print: |state| {
            let (status, draft) = match state {
                PrState::Open => ("active", false),
                PrState::Draft => ("active", true),
                PrState::Merged => ("completed", false),
                PrState::Closed => ("abandoned", false),
                PrState::Unknown => ("weird", false),
            };
            json!({
                "pullRequestId": 535,
                "status": status,
                "isDraft": draft,
                "repository": {"name": "Repo", "project": {"name": "Proj"}},
                "forkSource": null,
                "sourceRefName": "refs/heads/feature/x",
                "lastMergeSourceCommit": {"commitId": "ABCD1234"}
            })
            .to_string()
        },
        signed_out: (
            1,
            "ERROR: Please run 'az login' to setup account. secret-token-123",
        ),
        signed_out_message: "Azure CLI could not read the pull request; check Azure authentication with az login or az devops login.",
        not_installed_message: "Azure CLI (`az`) is not installed; install it and the azure-devops extension.",
    }
}

const KNOWN: [PrState; 4] = [
    PrState::Open,
    PrState::Draft,
    PrState::Merged,
    PrState::Closed,
];

#[test]
fn both_hosts_read_open_draft_merged_and_closed_in_one_call() {
    for case in [github(), azure()] {
        for state in KNOWN {
            let tools = Script::printing((case.print)(state));
            assert_eq!(
                Hosts::new(&tools).state(case.url),
                Ok(state),
                "{}",
                case.url
            );
            assert_eq!(*tools.calls.borrow(), [case.state_call]);
        }
    }
}

#[test]
fn both_hosts_read_the_head_with_its_state_in_one_call() {
    for case in [github(), azure()] {
        for state in KNOWN {
            let tools = Script::printing((case.print)(state));
            let head = Hosts::new(&tools).head(case.url).unwrap();
            let expected = Head {
                state,
                commit: "abcd1234".into(),
                branch: "feature/x".into(),
            };
            assert_eq!(head, expected, "{}", case.url);
            assert_eq!(tools.calls.borrow().len(), 1, "{}", case.url);
        }
    }
}

#[test]
fn a_state_the_host_reports_but_jidoka_does_not_know_is_a_problem() {
    for case in [github(), azure()] {
        let tools = Script::printing((case.print)(PrState::Unknown));
        let ToolProblem(message) = Hosts::new(&tools).state(case.url).unwrap_err();
        assert!(message.contains("unknown pull request"), "{message}");
        assert!(Hosts::new(&tools).head(case.url).is_err(), "{}", case.url);
    }
}

#[test]
fn a_signed_out_tool_says_how_to_sign_in_without_echoing_its_output() {
    for case in [github(), azure()] {
        let (code, stderr) = case.signed_out;
        let tools = Script::answering(Ok(ToolOutput {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.into(),
        }));
        let problem = Hosts::new(&tools).state(case.url).unwrap_err();
        assert_eq!(problem.0, case.signed_out_message);
    }
}

#[test]
fn a_missing_tool_says_how_to_install_it() {
    for case in [github(), azure()] {
        let tools = Script::missing();
        let problem = Hosts::new(&tools).head(case.url).unwrap_err();
        assert_eq!(problem.0, case.not_installed_message);
    }
}

#[test]
fn only_http_links_to_github_or_azure_pull_requests_reach_a_tool() {
    for url in [
        "--repo=evil",
        "file:///tmp/pull/7",
        "https://example.com/o/r/issues/7",
    ] {
        let tools = Script::missing();
        assert!(Hosts::new(&tools).state(url).is_err(), "{url}");
        assert!(tools.calls.borrow().is_empty(), "{url}");
    }
}
