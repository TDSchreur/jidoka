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

/// The earlier format: roles and tasks per agent.
const OLD_AGENTS: &str =
    r#"{"agents":{"claude":{"roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}}}}"#;
/// The earlier single-agent form.
const OLD_SINGLE: &str =
    r#"{"agent":"claude","roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}}"#;

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
    assert!(load_text(only_claude, Some(Host::Claude))
        .unwrap()
        .selection
        .is_some());
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
    assert!(load_text(text, Some(Host::Claude))
        .unwrap()
        .selection
        .is_some());
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
    assert!(load_text(&both, None)
        .unwrap_err()
        .contains("--agent copilot"));
}

#[test]
fn copilot_without_an_entry_says_so() {
    let resolution = load_text(MULTI, Some(Host::Copilot)).unwrap();
    assert_eq!(resolution.selection, None);
    assert!(resolution.notice.unwrap().contains("no entry for copilot"));
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
fn a_switched_off_policy_records_nothing_and_is_not_checked() {
    let multi = MULTI.replace("\"enabled\": true", "\"enabled\": false");
    let broken = r#"{"enabled":false,"agents":{"gemini":1},"agent":"claude"}"#;
    for text in [multi.as_str(), broken] {
        for host in [
            None,
            Some(Host::Claude),
            Some(Host::Codex),
            Some(Host::Copilot),
        ] {
            assert_eq!(
                load_text(text, host).unwrap(),
                Resolution::default(),
                "{text} {host:?}"
            );
        }
    }
}

#[test]
fn reading_never_changes_the_file() {
    let (_dir, path) = write(MULTI);
    read(&path).unwrap();
    load(&path, Some(Host::Claude)).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), MULTI);
    let (_dir, old) = write(OLD_AGENTS);
    read(&old).unwrap_err();
    assert_eq!(fs::read_to_string(&old).unwrap(), OLD_AGENTS);
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

// --- the earlier format -----------------------------------------------------------------------------

#[test]
fn an_old_format_file_fails_with_the_way_to_fix_it() {
    let roles_in_one_of_two = r#"{"agents":{
        "claude":{"stages":{"check":{"model":"opus"}}},
        "codex":{"roles":{"strong":{"model":"gpt-6-astra"}},"tasks":{}}}}"#;
    let tasks_only = r#"{"agents":{"claude":{"tasks":{"check":"strong"},"stages":{}}}}"#;
    for text in [
        OLD_AGENTS,
        OLD_SINGLE,
        r#"{"agent":"copilot"}"#,
        r#"{"roles":{},"tasks":{}}"#,
        roles_in_one_of_two,
        tasks_only,
    ] {
        for host in [None, Some(Host::Claude), Some(Host::Codex)] {
            let error = load_text(text, host).unwrap_err();
            for expected in [
                "old format",
                "factory install",
                "factory doctor --create-model-routing",
            ] {
                assert!(error.contains(expected), "{text} {host:?}: {error}");
            }
        }
    }
}

#[test]
fn old_format_text_is_told_apart_from_the_current_format_and_from_garbage() {
    for text in [OLD_AGENTS, OLD_SINGLE, r#"{"agent":"copilot"}"#] {
        assert!(is_old_format(text), "{text}");
    }
    for text in [
        MULTI,
        "not json",
        "[]",
        r#"{"agents":{"claude":{"typo":1}}}"#,
    ] {
        assert!(!is_old_format(text), "{text}");
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
fn a_problem_in_one_entry_names_the_entry_and_is_found_whichever_agent_the_run_is_in() {
    let broken_codex = r#"{"agents":{
        "claude":{"stages":{"check":{"model":"opus"}}},
        "codex":{"stages":{"build":{"model":"gpt 6"}}}}}"#;
    let error = read_error(broken_codex);
    assert!(error.contains("agents.codex"), "{error}");
    // A run in Claude stops too: a typo is found early, not only when that agent next starts a run.
    assert!(load_text(broken_codex, Some(Host::Claude))
        .unwrap_err()
        .contains("agents.codex"));
    let too_long = format!(
        r#"{{"agents":{{"claude":{{"stages":{{"check":{{"model":"{}"}}}}}}}}}}"#,
        "m".repeat(65)
    );
    assert!(read_error(&too_long).contains("agents.claude"));
}

#[test]
fn repeated_agents_and_stages_are_errors_instead_of_the_last_one_winning() {
    for (text, repeated) in [
        (
            r#"{"agents":{"claude":{"stages":{}},"claude":{"stages":{}}}}"#,
            "claude",
        ),
        (
            r#"{"agents":{"codex":{"stages":{"check":{"model":"a"},"check":{"model":"b"}}}}}"#,
            "check",
        ),
    ] {
        let error = read_error(text);
        assert!(
            error.contains("duplicate") && error.contains(repeated),
            "{text}: {error}"
        );
    }
}

#[test]
fn an_unreadable_or_oversized_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    fs::create_dir(&path).unwrap();
    assert!(load(&path, None).is_err());
    let big = dir.path().join("big.json");
    fs::write(&big, " ".repeat(1 << 20) + "{}").unwrap();
    assert!(load(&big, None).unwrap_err().contains("too large"));
}

#[test]
fn a_directory_at_the_policy_path_is_refused_as_not_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    fs::create_dir(&path).unwrap();
    assert!(read(&path).unwrap_err().contains("not a regular file"));
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

#[test]
fn a_created_starter_is_the_only_file_written() {
    // A failing write removes its partial file; that cannot be forced portably, so only the clean path is checked.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    assert!(create_starter(&path, &[RoutingAgent::Claude]).unwrap());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

// --- the --agent option -----------------------------------------------------------------------------

#[test]
fn the_agent_option_of_run_start_names_one_agent() {
    assert_eq!(parse_start_options(&[]), Ok(None));
    assert_eq!(
        parse_start_options(&["--agent", "claude"]),
        Ok(Some(Host::Claude))
    );
    assert_eq!(
        parse_start_options(&["--agent", "codex"]),
        Ok(Some(Host::Codex))
    );
    assert_eq!(
        parse_start_options(&["--agent", "copilot"]),
        Ok(Some(Host::Copilot))
    );
    for (bad, expected) in [
        (&["--agent"][..], "needs"),
        (&["--agent", "gemini"], "Unknown agent"),
        (&["--agent", "claude", "--agent", "codex"], "Unexpected"),
        (&["--agent", "claude", "extra"], "Unexpected"),
        (&["--session", "x"], "Unexpected"),
        (&["claude"], "Unexpected"),
    ] {
        let error = parse_start_options(bad).unwrap_err();
        assert!(error.contains(expected), "{bad:?}: {error}");
    }
}

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
            let effort = choice.effort.as_deref().expect("an effort");
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
fn a_starter_for_one_agent_has_only_that_entry() {
    let dir = tempfile::tempdir().unwrap();
    for (agent, other) in [
        (RoutingAgent::Claude, Host::Codex),
        (RoutingAgent::Codex, Host::Claude),
    ] {
        let path = dir.path().join(format!("{}.json", agent.name()));
        assert!(create_starter(&path, &[agent]).unwrap());
        assert_eq!(read(&path).unwrap().configured(), [agent.name()]);
        // The other agent has no entry, so it is not routed (and not given this one's policy).
        assert_eq!(load(&path, Some(other)).unwrap().selection, None);
    }
}

#[test]
fn a_starter_is_created_with_its_folder_and_never_replaces_anything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tds-factory").join("model-routing.json");
    assert!(create_starter(&path, &[RoutingAgent::Claude]).unwrap());
    assert!(fs::read_to_string(&path).unwrap().contains("\"agents\""));
    // A second call, even for another agent, leaves the file alone.
    fs::write(&path, "my own policy").unwrap();
    assert!(!create_starter(&path, &[RoutingAgent::Codex]).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), "my own policy");
}

#[cfg(unix)]
#[test]
fn a_link_at_the_policy_path_is_left_alone_even_when_it_points_nowhere() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere.json");
    let path = dir.path().join("model-routing.json");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(!create_starter(&path, &[RoutingAgent::Claude]).unwrap());
    assert!(
        !target.exists(),
        "the link must not be followed to create a file"
    );
}

// --- where the file lives ---------------------------------------------------------------------------

#[cfg(not(windows))]
#[test]
fn the_policy_lives_in_the_factorys_config_directory_and_ignores_a_relative_one() {
    let path = |vars: &'static [(&'static str, &'static str)]| {
        policy_path_in(move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).into())
        })
    };
    assert_eq!(
        path(&[("XDG_CONFIG_HOME", "/cfg"), ("HOME", "/home/u")]),
        Some(PathBuf::from("/cfg/tds-factory/model-routing.json"))
    );
    assert_eq!(
        path(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/u")]),
        Some(PathBuf::from(
            "/home/u/.config/tds-factory/model-routing.json"
        ))
    );
    // A relative XDG_CONFIG_HOME would put the file under whatever folder the command runs in.
    assert_eq!(
        path(&[("XDG_CONFIG_HOME", "relative/dir"), ("HOME", "/home/u")]),
        Some(PathBuf::from(
            "/home/u/.config/tds-factory/model-routing.json"
        ))
    );
    assert_eq!(
        path(&[("XDG_CONFIG_HOME", "relative/dir"), ("HOME", "home")]),
        None
    );
    assert_eq!(path(&[]), None);
}

#[cfg(windows)]
#[test]
fn the_policy_lives_in_the_factorys_config_directory() {
    let path =
        policy_path_in(|name| (name == "APPDATA").then(|| r"C:\Users\u\AppData\Roaming".into()));
    assert_eq!(
        path,
        Some(
            PathBuf::from(r"C:\Users\u\AppData\Roaming")
                .join("tds-factory")
                .join("model-routing.json")
        )
    );
}

#[test]
fn an_old_format_file_is_replaced_unless_it_is_oversized_or_not_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    let old = r#"{"agent":"claude","roles":{},"tasks":{}}"#;
    fs::write(&path, old).unwrap();
    assert_eq!(
        install_starter(&path, &[RoutingAgent::Claude]).unwrap(),
        Starter::Replaced(Off::default())
    );
    assert!(read(&path).is_ok());

    let big = format!("{{\"roles\":{{}},\"pad\":\"{}\"}}", "x".repeat(1 << 20));
    fs::write(&path, &big).unwrap();
    assert_eq!(
        install_starter(&path, &[RoutingAgent::Claude]).unwrap(),
        Starter::Left
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), big);
    let folder = dir.path().join("as-folder");
    fs::create_dir(&folder).unwrap();
    assert_eq!(
        install_starter(&folder, &[RoutingAgent::Claude]).unwrap(),
        Starter::Left
    );
}

fn replaced_starter(old: &str, agents: &[RoutingAgent]) -> serde_json::Value {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    fs::write(&path, old).unwrap();
    assert!(matches!(
        install_starter(&path, agents).unwrap(),
        Starter::Replaced(_)
    ));
    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
}

const BOTH: [RoutingAgent; 2] = [RoutingAgent::Claude, RoutingAgent::Codex];

#[test]
fn a_switched_off_old_file_is_replaced_by_a_switched_off_starter() {
    for old in [
        r#"{"enabled":false,"agent":"claude","roles":{}}"#,
        r#"{"enabled":false,"agents":{"claude":{"roles":{}}}}"#,
    ] {
        let value = replaced_starter(old, &BOTH);
        assert_eq!(value["enabled"], false, "{old}");
        assert!(value["agents"]["claude"].get("enabled").is_none(), "{old}");
        assert!(value["agents"]["codex"]["stages"]["build"].is_object());
    }
    // The switched-off starter is still a policy that reads, and a run start does not stop on it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    fs::write(&path, r#"{"enabled":false,"roles":{}}"#).unwrap();
    install_starter(&path, &BOTH).unwrap();
    assert!(matches!(read(&path).unwrap(), Policy::Off));
    assert_eq!(load(&path, Some(Host::Claude)).unwrap().selection, None);
}

#[test]
fn a_switched_off_agent_in_an_old_file_stays_off_in_the_starter() {
    let old = r#"{"agents":{
        "claude":{"enabled":false,"roles":{}},
        "codex":{"roles":{},"tasks":{}}}}"#;
    let value = replaced_starter(old, &BOTH);
    assert!(value.get("enabled").is_none_or(|on| on == true));
    assert_eq!(value["agents"]["claude"]["enabled"], false);
    assert!(value["agents"]["codex"].get("enabled").is_none());
    assert!(value["agents"]["claude"]["stages"]["define"].is_object());

    // Only the agents written to the starter carry a switch.
    let value = replaced_starter(old, &[RoutingAgent::Codex]);
    assert!(value["agents"].get("claude").is_none());
    assert!(value["agents"]["codex"].get("enabled").is_none());
}

#[test]
fn a_switched_on_old_file_gives_a_switched_on_starter_and_the_old_single_form_carries_only_the_top_switch(
) {
    let value = replaced_starter(r#"{"enabled":true,"roles":{}}"#, &BOTH);
    assert_eq!(value["enabled"], true);
    // The single-agent form has no agent entries to carry.
    let value = replaced_starter(
        r#"{"agent":"claude","roles":{},"agents":{"claude":{"enabled":false}}}"#,
        &BOTH,
    );
    assert_eq!(value["enabled"], true);
    assert!(value["agents"]["claude"].get("enabled").is_none());
}

#[test]
fn a_starter_keeps_the_stages_in_workflow_order_and_is_pretty_printed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    create_starter(&path, &BOTH).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("}\n") && text.lines().count() > 10, "{text}");
    let at = |stage: &str| text.find(&format!("\"{stage}\"")).unwrap();
    assert!(at("define") < at("build") && at("build") < at("check") && at("check") < at("finish"));
    assert!(text.find("\"claude\"").unwrap() < text.find("\"codex\"").unwrap());
}

#[cfg(unix)]
#[test]
fn a_link_to_an_old_format_file_is_not_followed_into_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere.json");
    let old = r#"{"roles":{},"tasks":{}}"#;
    fs::write(&target, old).unwrap();
    let path = dir.path().join("model-routing.json");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert_eq!(
        install_starter(&path, &[RoutingAgent::Claude]).unwrap(),
        Starter::Left
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), old);
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn a_replaced_outcome_carries_exactly_the_switches_written_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    let old = r#"{"enabled":false,"agents":{"claude":{"enabled":false,"roles":{}},"codex":{"enabled":false,"roles":{}}}}"#;
    fs::write(&path, old).unwrap();
    // The codex entry is not written, so it is not reported as off.
    let done = install_starter(&path, &[RoutingAgent::Claude]).unwrap();
    let off = Off {
        all: true,
        claude: true,
        codex: false,
    };
    assert_eq!(done, Starter::Replaced(off));
    let message = Starter::replaced_message("p", off, None);
    assert!(
        message.contains("whole policy") && message.contains("entry for claude"),
        "{message}"
    );
    assert!(
        !message.contains("codex") && !message.contains("stays off"),
        "{message}"
    );
}
