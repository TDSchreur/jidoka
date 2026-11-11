//! Behavior of the policy file through its public interface: `read`, `Policy::resolve`, `load`, the
//! `--agent` option and the starter. Every test writes into its own temporary folder.

use super::*;
use std::fs;

/// Both agents in one file, with settings that differ so a mix-up would show.
const MULTI: &str = r#"{
    "enabled": true,
    "agents": {
        "claude": {
            "stages": {"check": {"model": "opus", "effort": "high"}}
        },
        "codex": {
            "stages": {
                "define": {"model": "gpt-6-astra", "effort": "xhigh"},
                "build": {"model": "gpt-6.1-sol"},
                "finish": {"model": "gpt-6-luna", "effort": "low"}
            }
        }
    }
}"#;

fn write(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    fs::write(&path, text).unwrap();
    (dir, path)
}

fn load_text(text: &str, host: Option<Host>) -> Result<Resolution, String> {
    let (_dir, path) = write(text);
    load(&path, host)
}

fn selection(text: &str, host: Option<Host>) -> ModelRouting {
    load_text(text, host)
        .unwrap()
        .selection
        .expect("a selection")
}

/// The error for a file with this text, which must name the file.
fn read_error(text: &str) -> String {
    let (_dir, path) = write(text);
    let error = read(&path).unwrap_err();
    assert!(error.contains(&path.display().to_string()), "{error}");
    error
}

// --- one file for both agents ------------------------------------------------------------------------

#[test]
fn one_file_holds_independent_per_stage_settings_for_claude_and_codex() {
    let claude = selection(MULTI, Some(Host::Claude));
    assert_eq!(claude.agent, RoutingAgent::Claude);
    assert_eq!(claude.stages[&Stage::Check].model, "opus");
    assert_eq!(claude.stages[&Stage::Check].effort.as_deref(), Some("high"));
    // A Stage left out is not routed.
    assert_eq!(
        claude.stages.keys().copied().collect::<Vec<_>>(),
        [Stage::Check]
    );
    assert_eq!(claude.choice_for(Stage::Build), None);

    let codex = selection(MULTI, Some(Host::Codex));
    assert_eq!(codex.agent, RoutingAgent::Codex);
    assert_eq!(codex.stages[&Stage::Define].model, "gpt-6-astra");
    assert_eq!(
        codex.stages[&Stage::Define].effort.as_deref(),
        Some("xhigh")
    );
    // Effort is optional.
    assert_eq!(codex.stages[&Stage::Build].effort, None);
    assert_eq!(codex.stages.len(), 3);
    // Nothing of the other agent leaks into a selection, and no override is set.
    assert!(codex.overrides.is_empty() && claude.overrides.is_empty());
}

#[test]
fn a_run_records_only_the_selected_agents_stages() {
    let recorded = serde_json::to_value(selection(MULTI, Some(Host::Codex))).unwrap();
    assert_eq!(
        recorded,
        serde_json::json!({
            "agent": "codex",
            "stages": {
                "define": {"model": "gpt-6-astra", "effort": "xhigh"},
                "build": {"model": "gpt-6.1-sol"},
                "finish": {"model": "gpt-6-luna", "effort": "low"}
            }
        })
    );
}

#[test]
fn a_policy_for_several_agents_needs_to_be_told_which_one_the_run_is_in() {
    let error = load_text(MULTI, None).unwrap_err();
    for expected in [
        "claude, codex",
        "--agent claude",
        "--agent codex",
        "model-routing.json",
    ] {
        assert!(error.contains(expected), "{error}");
    }
    // With every entry switched off there is nothing to choose.
    let off = MULTI.replace("\"stages\"", "\"enabled\": false, \"stages\"");
    assert_eq!(load_text(&off, None).unwrap(), Resolution::default());
}

#[test]
fn an_agent_without_an_entry_is_not_routed_and_is_told_how_to_add_one() {
    let only_claude = r#"{"agents":{"claude":{"stages":{"check":{"model":"opus"}}}}}"#;
    let resolution = load_text(only_claude, Some(Host::Codex)).unwrap();
    assert_eq!(resolution.selection, None);
    let notice = resolution.notice.unwrap();
    assert!(
        notice.contains("no entry for codex") && notice.contains("\"agents\".\"codex\""),
        "{notice}"
    );
    // Claude's own entry is untouched by that.
    assert!(
        load_text(only_claude, Some(Host::Claude))
            .unwrap()
            .selection
            .is_some()
    );
}

#[test]
fn a_disabled_entry_routes_nothing_and_is_not_checked() {
    let text = r#"{"agents":{
        "claude":{"stages":{"check":{"model":"opus"}}},
        "codex":{"enabled":false,"stages":{"check":{"model":"not valid"}}}}}"#;
    assert_eq!(
        load_text(text, Some(Host::Codex)).unwrap(),
        Resolution::default()
    );
    assert!(
        load_text(text, Some(Host::Claude))
            .unwrap()
            .selection
            .is_some()
    );
}

const COPILOT: &str = r#"{"agents":{"copilot":{"stages":{
    "check":{"model":"claude-opus-4.8","effort":"xhigh"},
    "finish":{"model":"claude-haiku-4.5"}}}}}"#;

#[test]
fn a_copilot_run_records_the_copilot_entry_and_not_another_agents() {
    let both = COPILOT.replace(
        r#"{"agents":{"#,
        r#"{"agents":{"codex":{"stages":{"check":{"model":"gpt-6-astra"}}},"#,
    );
    for text in [COPILOT, both.as_str()] {
        let selection = load_text(text, Some(Host::Copilot))
            .unwrap()
            .selection
            .unwrap();
        assert_eq!(selection.agent, RoutingAgent::Copilot);
        assert_eq!(selection.stages.len(), 2);
        assert_eq!(selection.stages[&Stage::Check].model, "claude-opus-4.8");
        assert_eq!(
            selection.stages[&Stage::Check].effort.as_deref(),
            Some("xhigh")
        );
    }
    // The other agents get nothing from a file that only has a Copilot entry.
    let resolution = load_text(COPILOT, Some(Host::Claude)).unwrap();
    assert_eq!(resolution.selection, None);
    assert!(resolution.notice.unwrap().contains("no entry for claude"));
    // Several configured agents still need `--agent`.
    assert!(
        load_text(&both, None)
            .unwrap_err()
            .contains("--agent copilot")
    );
}

#[test]
fn copilot_entries_need_exact_model_ids_and_listed_efforts() {
    for (model, effort, problem) in [
        ("sonnet", "low", "Claude Code alias"),
        ("Haiku", "low", "Claude Code alias"),
        ("claude-sonnet-5.5", "ultra", "Effort"),
        ("claude-sonnet-5.5", "Low", "Effort"),
    ] {
        let text = format!(
            r#"{{"agents":{{"copilot":{{"stages":{{"check":{{"model":"{model}","effort":"{effort}"}}}}}}}}}}"#
        );
        let error = read_error(&text);
        assert!(
            error.contains("agents.copilot") && error.contains(problem),
            "{text}: {error}"
        );
    }
    for effort in ["none", "minimal", "low", "medium", "high", "xhigh", "max"] {
        let text = format!(
            r#"{{"agents":{{"copilot":{{"stages":{{"check":{{"model":"auto","effort":"{effort}"}}}}}}}}}}"#
        );
        assert!(
            load_text(&text, Some(Host::Copilot))
                .unwrap()
                .selection
                .is_some(),
            "{effort}"
        );
    }
    // Claude Code keeps its aliases, and a switched-off Copilot entry is not checked.
    assert!(
        load_text(MULTI.replace("opus", "sonnet").as_str(), Some(Host::Claude))
            .unwrap()
            .selection
            .is_some()
    );
    let off = r#"{"agents":{"copilot":{"enabled":false,"stages":{"check":{"model":"sonnet"}}}}}"#;
    assert_eq!(
        load_text(off, Some(Host::Copilot)).unwrap(),
        Resolution::default()
    );
}

#[test]
fn without_a_policy_file_nothing_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    assert_eq!(read(&path).unwrap(), Policy::Off);
    for host in [
        None,
        Some(Host::Claude),
        Some(Host::Codex),
        Some(Host::Copilot),
    ] {
        assert_eq!(load(&path, host).unwrap(), Resolution::default());
    }
}

// --- mistakes ---------------------------------------------------------------------------------------

#[test]
fn mistakes_in_the_file_are_errors_that_name_the_file_and_the_problem() {
    for (text, problem) in [
        ("not json", "not valid JSON"),
        ("[]", "invalid"),
        // Unknown fields, Stages and agents, at each level.
        (r#"{"agents":{"claude":{"stages":{}}},"extra":1}"#, "extra"),
        (r#"{"agents":{"claude":{"stages":{},"extra":1}}}"#, "extra"),
        (
            r#"{"agents":{"claude":{"stages":{"check":{"model":"a","extra":1}}}}}"#,
            "extra",
        ),
        (
            r#"{"agents":{"claude":{"stages":{"deploy":{"model":"a"}}}}}"#,
            "deploy",
        ),
        (r#"{"agents":{"gemini":{"stages":{}}}}"#, "gemini"),
        // Unsupported or empty.
        (r#"{"agents":{}}"#, "no entry"),
        (r#"{"agents":[]}"#, "invalid"),
        // Missing parts.
        (r#"{}"#, "agents"),
        (r#"{"agents":{"claude":{}}}"#, "stages"),
        (r#"{"agents":{"claude":{"stages":{"check":{}}}}}"#, "model"),
        // Values.
        (
            r#"{"agents":{"claude":{"stages":{"check":{"model":"a b"}}}}}"#,
            "Model",
        ),
        (
            r#"{"agents":{"claude":{"stages":{"check":{"model":"a","effort":""}}}}}"#,
            "Effort",
        ),
    ] {
        let error = read_error(text);
        assert!(error.contains(problem), "{text}: {error}");
    }
}

#[test]
fn a_broken_policy_stops_a_run_in_any_agent() {
    let broken = r#"{"agents": {"claude": {"typo": 1}}}"#;
    for host in [
        None,
        Some(Host::Claude),
        Some(Host::Codex),
        Some(Host::Copilot),
    ] {
        assert!(load_text(broken, host).is_err(), "{host:?}");
    }
}

// --- the --agent option -----------------------------------------------------------------------------

// --- the starter ------------------------------------------------------------------------------------

#[test]
fn a_starter_for_both_agents_routes_every_stage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    assert!(create_starter(&path, &[RoutingAgent::Claude, RoutingAgent::Codex]).unwrap());
    assert!(matches!(
        read(&path).unwrap(),
        Policy::Multi {
            claude: Some(_),
            codex: Some(_),
            copilot: None
        }
    ));
    for (host, agent) in [
        (Host::Claude, RoutingAgent::Claude),
        (Host::Codex, RoutingAgent::Codex),
    ] {
        let recorded = load(&path, Some(host))
            .unwrap()
            .selection
            .expect("the starter routes");
        assert_eq!(recorded.agent, agent);
        assert_eq!(recorded.stages.len(), 4);
        for stage in [Stage::Define, Stage::Build, Stage::Check, Stage::Finish] {
            let choice = recorded
                .choice_for(stage)
                .unwrap_or_else(|| panic!("{agent:?} {stage:?} is routed"));
            assert!(!choice.model.is_empty(), "{agent:?} {stage:?}");
            let effort = choice
                .effort
                .as_deref()
                .unwrap_or_else(|| panic!("{agent:?} {stage:?} has an effort: {choice:?}"));
            assert!(
                ["low", "medium", "high", "xhigh", "max"].contains(&effort),
                "{agent:?} {stage:?}: {effort}"
            );
        }
    }
    // A run must name its agent.
    assert!(load(&path, None).is_err());
}

#[test]
fn a_starter_is_created_with_its_folder_and_never_replaces_anything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jidoka").join("model-routing.json");
    assert!(create_starter(&path, &[RoutingAgent::Claude]).unwrap());
    assert!(fs::read_to_string(&path).unwrap().contains("\"agents\""));
    // A second call, even for another agent, leaves the file alone.
    fs::write(&path, "my own policy").unwrap();
    assert!(!create_starter(&path, &[RoutingAgent::Codex]).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), "my own policy");
}
