//! Behavior of the policy file through its public interface: `read`, `Policy::resolve`, `load`, the
//! `--agent` option and the starter. Every test writes into its own temporary folder.

use super::*;
use std::fs;

/// The single-agent form, as it has always been written.
const SINGLE_CLAUDE: &str = r#"{
    "enabled": true,
    "agent": "claude",
    "roles": {
        "strong": {"model": "opus", "effort": "high"},
        "coding": {"model": "sonnet", "effort": "medium"},
        "routine": {"model": "haiku", "effort": "low"}
    },
    "tasks": {"build": "coding", "check": "strong", "finish": "routine"}
}"#;

/// Both agents in one file, with settings that differ so a mix-up would show.
const MULTI: &str = r#"{
    "enabled": true,
    "agents": {
        "claude": {
            "roles": {"strong": {"model": "opus", "effort": "high"}},
            "tasks": {"check": "strong"}
        },
        "codex": {
            "roles": {
                "strong": {"model": "gpt-6-astra", "effort": "xhigh"},
                "coding": {"model": "gpt-6.1-sol"}
            },
            "tasks": {"check": "strong", "build": "coding"}
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
fn one_file_holds_independent_settings_for_claude_and_codex() {
    let claude = selection(MULTI, Some(Host::Claude));
    assert_eq!(claude.agent, RoutingAgent::Claude);
    assert_eq!(claude.roles[&Role::Strong].model, "opus");
    assert_eq!(claude.roles.len(), 1);
    assert_eq!(
        claude.tasks.keys().copied().collect::<Vec<_>>(),
        [Task::Check]
    );

    let codex = selection(MULTI, Some(Host::Codex));
    assert_eq!(codex.agent, RoutingAgent::Codex);
    assert_eq!(codex.roles[&Role::Strong].model, "gpt-6-astra");
    assert_eq!(codex.roles[&Role::Strong].effort.as_deref(), Some("xhigh"));
    assert_eq!(codex.roles[&Role::Coding].effort, None);
    assert_eq!(codex.role_for(Task::Build), Some(Role::Coding));
    // Nothing of the other agent leaks into a selection, and no override is set.
    assert!(codex.overrides.is_empty() && claude.overrides.is_empty());
}

#[test]
fn a_run_records_only_the_selected_agents_policy_in_the_existing_run_form() {
    let recorded = serde_json::to_value(selection(MULTI, Some(Host::Codex))).unwrap();
    assert_eq!(
        recorded,
        serde_json::json!({
            "agent": "codex",
            "roles": {"strong": {"model": "gpt-6-astra", "effort": "xhigh"}, "coding": {"model": "gpt-6.1-sol"}},
            "tasks": {"build": "coding", "check": "strong"}
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
    // With every entry switched off there is nothing to choose, so the old invocation still works.
    let off = MULTI.replace("\"roles\"", "\"enabled\": false, \"roles\"");
    assert_eq!(load_text(&off, None).unwrap(), Resolution::default());
}

#[test]
fn an_agent_without_an_entry_is_not_routed_and_is_told_how_to_add_one() {
    let only_claude =
        r#"{"agents":{"claude":{"roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}}}}"#;
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
        "claude":{"roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}},
        "codex":{"enabled":false,"roles":{},"tasks":{"check":"strong"}}}}"#;
    assert_eq!(
        load_text(text, Some(Host::Codex)).unwrap(),
        Resolution::default()
    );
    assert!(load_text(text, Some(Host::Claude))
        .unwrap()
        .selection
        .is_some());
}

#[test]
fn copilot_routes_nothing_whatever_the_form() {
    for text in [MULTI, SINGLE_CLAUDE, r#"{"agent":"copilot"}"#] {
        let resolution = load_text(text, Some(Host::Copilot)).unwrap();
        assert_eq!(resolution.selection, None, "{text}");
        let notice = resolution.notice.unwrap();
        assert!(
            notice.contains("Copilot") && notice.contains("session"),
            "{notice}"
        );
    }
}

#[test]
fn a_switched_off_policy_records_nothing_and_is_not_checked_in_either_form() {
    let single = SINGLE_CLAUDE.replace("\"enabled\": true", "\"enabled\": false");
    let multi = MULTI.replace("\"enabled\": true", "\"enabled\": false");
    let broken = r#"{"enabled":false,"agents":{"gemini":1},"agent":"claude"}"#;
    for text in [single.as_str(), multi.as_str(), broken] {
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

// --- the single-agent form keeps working ------------------------------------------------------------

#[test]
fn an_existing_single_agent_policy_is_recorded_with_and_without_an_agent_option() {
    for host in [None, Some(Host::Claude)] {
        let recorded = selection(SINGLE_CLAUDE, host);
        assert_eq!(recorded.agent, RoutingAgent::Claude);
        assert_eq!(recorded.roles[&Role::Strong].model, "opus");
        assert_eq!(
            recorded.roles[&Role::Routine].effort.as_deref(),
            Some("low")
        );
        assert_eq!(recorded.tasks[&Task::Finish], Role::Routine);
    }
    let codex = r#"{"agent":"codex","roles":{"coding":{"model":"gpt-6.1-sol"}},"tasks":{"build":"coding"}}"#;
    let recorded = selection(codex, None);
    assert_eq!(recorded.agent, RoutingAgent::Codex);
    assert_eq!(recorded.roles[&Role::Coding].effort, None);
    assert_eq!(recorded.tasks.get(&Task::Check), None);
}

#[test]
fn a_single_agent_policy_is_never_used_for_the_other_agent() {
    let resolution = load_text(SINGLE_CLAUDE, Some(Host::Codex)).unwrap();
    assert_eq!(resolution.selection, None);
    let notice = resolution.notice.unwrap();
    assert!(
        notice.contains("for claude")
            && notice.contains("codex run")
            && notice.contains("\"agents\""),
        "{notice}"
    );
}

#[test]
fn a_bare_copilot_policy_gets_the_notice_instead_of_a_parse_error() {
    let resolution = load_text(r#"{"agent":"copilot"}"#, None).unwrap();
    assert_eq!(resolution.selection, None);
    assert!(resolution.notice.unwrap().contains("Copilot"));
}

#[test]
fn reading_never_changes_the_file() {
    let (_dir, path) = write(SINGLE_CLAUDE);
    read(&path).unwrap();
    load(&path, Some(Host::Claude)).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), SINGLE_CLAUDE);
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
        // The two forms at once.
        (
            r#"{"agent":"claude","roles":{},"tasks":{},"agents":{"codex":{"roles":{},"tasks":{}}}}"#,
            "mixes",
        ),
        (
            r#"{"roles":{},"tasks":{},"agents":{"codex":{"roles":{},"tasks":{}}}}"#,
            "mixes",
        ),
        // Unknown fields and agents, at each level.
        (
            r#"{"agent":"claude","roles":{},"tasks":{},"extra":1}"#,
            "extra",
        ),
        (
            r#"{"agents":{"claude":{"roles":{},"tasks":{},"extra":1}}}"#,
            "extra",
        ),
        (r#"{"agents":{"gemini":{"roles":{},"tasks":{}}}}"#, "gemini"),
        (r#"{"agent":"gemini","roles":{},"tasks":{}}"#, "gemini"),
        // Unsupported or empty.
        (
            r#"{"agents":{"copilot":{"roles":{},"tasks":{}}}}"#,
            "Copilot CLI",
        ),
        (r#"{"agents":{}}"#, "no entry"),
        (r#"{"agents":[]}"#, "invalid"),
        // Missing parts of either form.
        (r#"{"roles":{},"tasks":{}}"#, "agent"),
        (r#"{"agent":"claude","tasks":{}}"#, "roles"),
        (r#"{"agent":"claude","roles":{}}"#, "tasks"),
        (r#"{"agents":{"claude":{"tasks":{}}}}"#, "roles"),
        // Values.
        (
            r#"{"agent":"claude","roles":{"coding":{"model":"sonnet"}},"tasks":{"check":"strong"}}"#,
            "strong",
        ),
        (
            r#"{"agent":"claude","roles":{"coding":{"model":"a b"}},"tasks":{}}"#,
            "Model",
        ),
    ] {
        let error = read_error(text);
        assert!(error.contains(problem), "{text}: {error}");
    }
}

#[test]
fn a_problem_in_one_entry_names_the_entry_and_is_found_whichever_agent_the_run_is_in() {
    let broken_codex = r#"{"agents":{
        "claude":{"roles":{"strong":{"model":"opus"}},"tasks":{"check":"strong"}},
        "codex":{"roles":{"strong":{"model":"gpt-6-astra"}},"tasks":{"check":"coding"}}}}"#;
    let error = read_error(broken_codex);
    assert!(
        error.contains("agents.codex") && error.contains("coding"),
        "{error}"
    );
    // A run in Claude stops too: a typo is found early, not only when that agent next starts a run.
    assert!(load_text(broken_codex, Some(Host::Claude))
        .unwrap_err()
        .contains("agents.codex"));
    let bad_text = r#"{"agents":{"claude":{"roles":{"strong":{"model":"op us"}},"tasks":{}}}}"#;
    assert!(read_error(bad_text).contains("agents.claude"));
}

#[test]
fn repeated_agents_roles_and_tasks_are_errors_instead_of_the_last_one_winning() {
    for (text, repeated) in [
        (
            r#"{"agents":{"claude":{"roles":{"strong":{"model":"a"}},"tasks":{}},"claude":{"roles":{"strong":{"model":"b"}},"tasks":{}}}}"#,
            "claude",
        ),
        (
            r#"{"agent":"claude","roles":{"strong":{"model":"a"},"strong":{"model":"b"}},"tasks":{}}"#,
            "strong",
        ),
        (
            r#"{"agents":{"codex":{"roles":{"strong":{"model":"a"}},"tasks":{"check":"strong","check":"strong"}}}}"#,
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
fn a_broken_policy_cannot_stop_a_copilot_run_but_still_stops_the_others() {
    let broken = r#"{"agents": {"claude": {"typo": 1}}}"#;
    let resolution = load_text(broken, Some(Host::Copilot)).unwrap();
    assert!(resolution.selection.is_none());
    assert!(load_text(broken, Some(Host::Claude)).is_err());
    assert!(load_text(broken, None).is_err());
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
fn a_starter_for_both_agents_is_one_usable_file_with_an_entry_each() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model-routing.json");
    assert!(create_starter(&path, &[RoutingAgent::Claude, RoutingAgent::Codex]).unwrap());
    assert!(matches!(
        read(&path).unwrap(),
        Policy::Multi {
            claude: Some(_),
            codex: Some(_)
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
        // Only check is routed, to the strong role; all three roles are there to extend.
        assert_eq!(
            recorded.tasks.keys().copied().collect::<Vec<_>>(),
            [Task::Check]
        );
        assert_eq!(recorded.role_for(Task::Check), Some(Role::Strong));
        for role in [Role::Strong, Role::Coding, Role::Routine] {
            let choice = &recorded.roles[&role];
            assert!(
                !choice.model.is_empty() && choice.effort.is_some(),
                "{agent:?} {role:?}"
            );
        }
    }
    // The two entries are different, and a run must name its agent.
    let claude = selection(&fs::read_to_string(&path).unwrap(), Some(Host::Claude));
    let codex = selection(&fs::read_to_string(&path).unwrap(), Some(Host::Codex));
    assert_ne!(
        claude.roles[&Role::Strong].model,
        codex.roles[&Role::Strong].model
    );
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
    let path = dir.path().join("tds-factory/model-routing.json");
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
