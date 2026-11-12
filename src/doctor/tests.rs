use super::*;
use crate::install::bundled_skill_names;
use std::collections::HashMap;
use tempfile::TempDir;

/// A project, a home folder and a `PATH` folder, all temporary.
struct World {
    project: TempDir,
    home: TempDir,
    bin: TempDir,
    vars: HashMap<String, OsString>,
    /// What each agent's program prints for `--version`.
    versions: HashMap<String, String>,
    /// What `rustup target list --installed` prints; `None` when rustup is not available.
    rustup_targets: Option<String>,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn program(bin: &Path, name: &str) {
    let file = bin.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    });
    fs::write(&file, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn install_skills(root: &Path) {
    for file in bundled_skill_files() {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "file").unwrap();
    }
}

const AGENTS_MD: &str = "# Project\n\n## Commands\n\n- Test: `cargo test`\n- Lint: `cargo clippy -- -D warnings`\n\n## Agent skills\n\nSee docs.\n";

impl World {
    /// A ready project (repository, origin, base branch, setup docs, commands) with `cargo` on PATH and nothing else.
    fn ready() -> Self {
        let world = Self {
            project: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
            bin: tempfile::tempdir().unwrap(),
            vars: HashMap::new(),
            versions: HashMap::new(),
            rustup_targets: None,
        };
        let dir = world.project.path();
        git(dir, &["init", "-q"]);
        git(
            dir,
            &["remote", "add", "origin", "https://example.com/repo.git"],
        );
        git(
            dir,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ],
        );
        fs::create_dir_all(dir.join("docs/agents")).unwrap();
        fs::write(dir.join("docs/agents/issue-tracker.md"), "tracker").unwrap();
        fs::write(dir.join("AGENTS.md"), AGENTS_MD).unwrap();
        program(world.bin.path(), "cargo");
        world
    }

    /// Makes `agent` usable: its command on PATH and its skills in the default folder.
    fn with_agent(self, agent: &str) -> Self {
        program(self.bin.path(), agent);
        let root =
            skills_root(agent, self.home.path(), |name| self.vars.get(name).cloned()).unwrap();
        install_skills(&root);
        self
    }

    fn report(&self, agent: Option<&str>) -> Report {
        self.report_in(agent, &[]).unwrap()
    }

    fn report_in(&self, agent: Option<&str>, config_dirs: &[&str]) -> io::Result<Report> {
        let machine = Machine {
            home: self.home.path().to_path_buf(),
            path: self.bin.path().into(),
            var: &|name| self.vars.get(name).cloned(),
            config_dirs: config_dirs.iter().map(|dir| dir.to_string()).collect(),
            version: &|program| {
                let name = program.file_stem()?.to_str()?;
                self.versions.get(name).cloned()
            },
            rustup_targets: &|_, _| self.rustup_targets.clone(),
        };
        diagnose(self.project.path(), agent, &machine)
    }
}

fn project_check<'a>(report: &'a Report, id: &str) -> &'a Check {
    report
        .project
        .iter()
        .find(|check| check.id == id)
        .unwrap_or_else(|| panic!("no {id} check"))
}

fn agent<'a>(report: &'a Report, id: &str) -> &'a AgentReport {
    report.agents.iter().find(|a| a.id == id).unwrap()
}

#[test]
fn a_ready_project_with_one_usable_agent_passes() {
    let report = World::ready().with_agent("claude").report(None);
    assert!(report.ok, "{}", render(&report));
    assert!(report.project.iter().all(Check::passed));
    assert!(agent(&report, "claude").usable);
    assert!(!agent(&report, "codex").usable);
    assert!(
        project_check(&report, "base-branch")
            .message
            .contains("develop")
    );
}

#[test]
fn without_any_usable_agent_the_report_is_not_ok_and_says_how_to_fix_each() {
    let report = World::ready().report(None);
    assert!(!report.ok);
    for a in &report.agents {
        assert!(!a.usable);
        let fixes: Vec<_> = a.checks.iter().filter_map(|c| c.fix.as_deref()).collect();
        assert!(
            fixes
                .iter()
                .any(|fix| fix.contains(&format!("jidoka install --agent {}", a.id))),
            "{fixes:?}"
        );
    }
}

#[test]
fn an_agent_needs_both_its_command_and_every_bundled_skill() {
    let world = World::ready().with_agent("claude");
    let skill = skills_root("claude", world.home.path(), |_| None)
        .unwrap()
        .join(bundled_skill_names()[0]);
    fs::remove_dir_all(skill).unwrap();
    let report = world.report(Some("claude"));
    assert!(!report.ok);
    let skills = &agent(&report, "claude").checks[1];
    assert_eq!(skills.status, Status::Missing);
    assert!(skills.message.contains(bundled_skill_names()[0]));

    // A skill that lost one of its other files is incomplete too.
    let world = World::ready().with_agent("claude");
    let root = skills_root("claude", world.home.path(), |_| None).unwrap();
    let extra = bundled_skill_files()
        .find(|file| !file.ends_with("/SKILL.md"))
        .expect("a skill ships more than SKILL.md");
    fs::remove_file(root.join(extra)).unwrap();
    let report = world.report(Some("claude"));
    assert_eq!(agent(&report, "claude").checks[1].status, Status::Missing);
    assert!(
        agent(&report, "claude").checks[1]
            .message
            .contains(extra.split('/').next().unwrap())
    );

    let world = World::ready();
    install_skills(&skills_root("codex", world.home.path(), |_| None).unwrap());
    let report = world.report(Some("codex"));
    assert!(!report.ok);
    assert_eq!(
        agent(&report, "codex").checks[0].status,
        Status::Missing,
        "no codex command"
    );
}

#[test]
fn skills_are_looked_up_where_install_would_put_them() {
    let mut world = World::ready();
    let custom = tempfile::tempdir().unwrap();
    world
        .vars
        .insert("CLAUDE_CONFIG_DIR".into(), custom.path().into());
    world
        .vars
        .insert("CODEX_HOME".into(), custom.path().join("codex").into());
    world
        .vars
        .insert("COPILOT_HOME".into(), custom.path().join("copilot").into());
    let world = world
        .with_agent("claude")
        .with_agent("codex")
        .with_agent("copilot");
    assert!(
        custom
            .path()
            .join("skills")
            .join(bundled_skill_names()[0])
            .is_dir()
    );
    assert!(custom.path().join("codex/skills").is_dir());
    assert!(custom.path().join("copilot/skills").is_dir());
    assert!(!world.home.path().join(".claude").exists());
    assert!(!world.home.path().join(".copilot").exists());
    let report = world.report(None);
    assert!(agent(&report, "claude").usable);
    assert!(agent(&report, "codex").usable);
    assert!(agent(&report, "copilot").usable);
}

#[test]
fn project_problems_are_named_with_a_fix_and_make_the_report_not_ok() {
    // Nothing but an empty folder.
    let world = World::ready().with_agent("claude");
    let empty = tempfile::tempdir().unwrap();
    let machine = Machine {
        home: world.home.path().into(),
        path: world.bin.path().into(),
        var: &|_| None,
        config_dirs: Vec::new(),
        version: &|_| None,
        rustup_targets: &|_, _| None,
    };
    let report = diagnose(empty.path(), Some("claude"), &machine).unwrap();
    assert!(!report.ok);
    assert_eq!(project_check(&report, "git").status, Status::Fail);

    let no_origin = World::ready().with_agent("claude");
    git(no_origin.project.path(), &["remote", "remove", "origin"]);
    let report = no_origin.report(None);
    assert!(!report.ok);
    assert_eq!(project_check(&report, "origin").status, Status::Fail);
    assert!(
        project_check(&report, "origin")
            .fix
            .as_ref()
            .unwrap()
            .contains("git remote add origin")
    );

    let no_base = World::ready().with_agent("claude");
    git(
        no_base.project.path(),
        &["symbolic-ref", "--delete", "refs/remotes/origin/HEAD"],
    );
    let report = no_base.report(None);
    assert!(!report.ok);
    assert_eq!(
        project_check(&report, "base-branch").status,
        Status::Missing
    );
    assert!(
        project_check(&report, "base-branch")
            .fix
            .as_ref()
            .unwrap()
            .contains("set-head")
    );
}

#[test]
fn doctor_changes_nothing() {
    fn snapshot(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                snapshot(&path, out);
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let world = World::ready().with_agent("claude");
    let take = || {
        let mut all = Vec::new();
        for dir in [world.project.path(), world.home.path(), world.bin.path()] {
            snapshot(dir, &mut all);
        }
        all
    };
    let before = take();
    world.report(None);
    assert_eq!(before, take());
}

#[test]
fn the_json_report_has_lowercase_statuses_and_fixes_only_where_needed() {
    let report = World::ready().with_agent("claude").report(None);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["project"][0]["id"], "git");
    assert_eq!(json["project"][0]["status"], "pass");
    assert!(json["project"][0].get("fix").is_none());
    let codex = json["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "codex")
        .unwrap();
    assert_eq!(codex["usable"], false);
    assert_eq!(codex["checks"][0]["status"], "missing");
    assert!(
        codex["checks"][0]["fix"]
            .as_str()
            .unwrap()
            .contains("codex")
    );
}

fn write_record(world: &mut World, version: &str) {
    let base = world.home.path().join("cfg");
    world
        .vars
        .insert("XDG_CONFIG_HOME".into(), base.clone().into());
    world.vars.insert("APPDATA".into(), base.into());
    let path = crate::install_record::path_in(|name| world.vars.get(name).cloned()).unwrap();
    let record = crate::install_record::InstallRecord {
        version: version.into(),
        agents: vec![],
        claude_config_dirs: vec![],
    };
    crate::install_record::write(&path, &record).unwrap();
}

#[test]
fn a_record_behind_the_binary_is_reported_outdated_with_the_fix() {
    for version in ["0.0.1", ""] {
        let mut world = World::ready().with_agent("claude");
        write_record(&mut world, version);
        let report = world.report(Some("claude"));
        let check = project_check(&report, "skills-version");
        assert_eq!(check.status, Status::Warning, "{version:?}");
        assert!(check.message.contains("outdated"));
        assert!(check.message.contains(env!("CARGO_PKG_VERSION")));
        let fix = check.fix.as_deref().unwrap();
        assert!(fix.contains("jidoka install") && fix.contains("any `jidoka` command"));
        assert!(report.ok, "a warning never blocks");
        let json = serde_json::to_value(&report).unwrap();
        assert!(
            json["project"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| { c["id"] == "skills-version" && c["status"] == "warning" })
        );
    }
}

#[test]
fn the_text_report_shows_status_and_fix_per_line() {
    let text = render(&World::ready().with_agent("claude").report(None));
    assert!(text.contains("pass     git:"));
    assert!(text.contains("missing  executable: `codex` is not on PATH"));
    assert!(text.contains("Fix: Run `jidoka install --agent codex`."));
    assert!(text.contains("claude: usable") && text.contains("codex: not usable"));
    assert!(text.contains("Ready"));
}

#[test]
fn creating_the_policy_covers_every_agent_unless_one_is_named() {
    let error = |args: &[&str]| parse_args(args).unwrap_err();
    let both = parse_args(&["--create-model-routing"]).unwrap();
    assert_eq!(
        both.create_model_routing,
        Some(vec![
            RoutingAgent::Claude,
            RoutingAgent::Codex,
            RoutingAgent::Copilot
        ])
    );
    let copilot = parse_args(&["--create-model-routing", "--agent", "copilot"]).unwrap();
    assert_eq!(
        copilot.create_model_routing,
        Some(vec![RoutingAgent::Copilot])
    );
    assert!(error(&["--create-model-routing", "--agent", "gemini"]).contains("Unknown agent"));
}

impl World {
    /// What `--create-model-routing` does on this fake machine.
    fn create_model_routing(&self, agents: &[RoutingAgent]) -> Result<String, String> {
        let machine = Machine {
            home: self.home.path().to_path_buf(),
            path: self.bin.path().into(),
            var: &|name| self.vars.get(name).cloned(),
            config_dirs: Vec::new(),
            version: &|_| None,
            rustup_targets: &|_, _| None,
        };
        create_model_routing(agents, &machine)
    }
}

#[test]
fn doctor_creates_a_starter_policy_for_the_agent_and_then_checks_it() {
    let world = World::ready().claude_ready().with_config_folder();
    let message = world.create_model_routing(&[RoutingAgent::Claude]).unwrap();
    // Built part by part, so the separators are the ones the message shows on every system.
    let path = world
        .home
        .path()
        .join("config")
        .join("jidoka")
        .join("model-routing.json");
    assert!(path.is_file(), "{message}");
    assert!(message.contains(&path.display().to_string()), "{message}");
    assert!(
        message.contains("check") && message.contains("enabled"),
        "{message}"
    );
    // The next report already sees the file, and it passes every check.
    let report = world.report(None);
    assert!(report.ok, "{}", render(&report));
    assert!(
        report.model_routing.iter().all(Check::passed),
        "{}",
        render(&report)
    );
    // Created for Codex, the policy names Codex.
    let read = |world: &World| {
        fs::read_to_string(world.home.path().join("config/jidoka/model-routing.json")).unwrap()
    };
    let text = read(&world);
    assert!(
        text.contains("\"claude\"") && !text.contains("\"codex\""),
        "{text}"
    );
    // For Codex only, and for both agents in one file.
    let codex = World::ready().with_config_folder();
    codex.create_model_routing(&[RoutingAgent::Codex]).unwrap();
    let text = read(&codex);
    assert!(
        text.contains("\"codex\"") && !text.contains("\"claude\""),
        "{text}"
    );
    let both = World::ready().with_config_folder();
    let message = both
        .create_model_routing(&[RoutingAgent::Claude, RoutingAgent::Codex])
        .unwrap();
    assert!(message.contains("claude and codex"), "{message}");
    let text = read(&both);
    assert!(
        text.contains("\"claude\"") && text.contains("\"codex\""),
        "{text}"
    );
}

#[test]
fn doctor_leaves_an_existing_policy_alone_and_says_so() {
    let world = World::ready().claude_ready().with_policy(CLAUDE_POLICY);
    let path = world.home.path().join("config/jidoka/model-routing.json");
    let message = world.create_model_routing(&[RoutingAgent::Claude]).unwrap();
    assert!(
        message.contains("already exists") && message.contains("unchanged"),
        "{message}"
    );
    assert_eq!(fs::read_to_string(path).unwrap(), CLAUDE_POLICY);
}

// --- one policy file for several agents ---------------------------------------------------------

/// Claude and Codex in one file, with different models.
const BOTH_POLICY: &str = r#"{"agents":{
    "claude":{"stages":{"check":{"model":"opus","effort":"high"}}},
    "codex":{"stages":{"check":{"model":"gpt-6-astra","effort":"high"}}}}}"#;

fn tagged<'a>(report: &'a Report, agent: &str) -> Vec<&'a Check> {
    report
        .model_routing
        .iter()
        .filter(|c| c.agent == Some(agent))
        .collect()
}

#[test]
fn checking_all_agents_reports_each_configured_entry_tagged_with_its_agent() {
    let world = World::ready()
        .claude_ready()
        .with_agent("codex")
        .with_version("codex", "codex-cli 0.166.0")
        .with_policy(BOTH_POLICY);
    let report = world.report(None);
    assert!(report.ok, "{}", render(&report));
    assert_eq!(report.model_routing[0].id, "policy");
    assert!(
        report.model_routing[0]
            .message
            .contains("configures claude, codex")
    );
    assert_eq!(report.model_routing[0].agent, None);
    let claude: Vec<_> = tagged(&report, "claude").iter().map(|c| c.id).collect();
    assert_eq!(claude, ["host", "model", "effort", "overrides"]);
    // Codex has its own checks, which skip what Jidoka cannot judge.
    let codex = tagged(&report, "codex");
    assert_eq!(codex.len(), 3);
    assert!(
        codex
            .iter()
            .any(|c| c.id == "model" && c.status == Status::Skipped)
    );
    let text = render(&report);
    assert!(text.contains("pass     host [claude]:"), "{text}");
    assert!(text.contains("skipped  model [codex]:"), "{text}");
    let json = serde_json::to_value(&report).unwrap();
    assert!(
        json["model_routing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["agent"] == "codex")
    );
}

// --- model routing -----------------------------------------------------------------------------

const CLAUDE_POLICY: &str = r#"{"agents":{"claude":{"stages":{
    "build":{"model":"sonnet","effort":"medium"},
    "check":{"model":"opus","effort":"high"}}}}}"#;

impl World {
    /// Writes the user's model-routing policy where doctor looks for it, inside the fake home.
    fn with_policy(mut self, policy: &str) -> Self {
        let config = self.home.path().join("config");
        fs::create_dir_all(config.join("jidoka")).unwrap();
        fs::write(config.join("jidoka").join("model-routing.json"), policy).unwrap();
        for name in ["XDG_CONFIG_HOME", "APPDATA"] {
            self.vars.insert(name.into(), config.clone().into());
        }
        self
    }

    /// Points the configuration folder at a temporary one inside the fake home, with no policy in it.
    fn with_config_folder(mut self) -> Self {
        let config = self.home.path().join("config");
        for name in ["XDG_CONFIG_HOME", "APPDATA"] {
            self.vars.insert(name.into(), config.clone().into());
        }
        self
    }

    fn with_version(mut self, agent: &str, output: &str) -> Self {
        self.versions.insert(agent.into(), output.into());
        self
    }

    fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }

    fn claude_ready(self) -> Self {
        self.with_agent("claude")
            .with_version("claude", "2.1.318 (Claude Code)")
    }
}

fn routing_check<'a>(report: &'a Report, id: &str) -> &'a Check {
    report
        .model_routing
        .iter()
        .find(|check| check.id == id)
        .unwrap_or_else(|| panic!("no {id} model-routing check in {report:?}"))
}

#[test]
fn without_a_policy_there_is_no_model_routing_section() {
    let report = World::ready().claude_ready().report(None);
    assert!(report.model_routing.is_empty());
    let json = serde_json::to_value(&report).unwrap();
    assert!(json.get("model_routing").is_none());
    assert!(!render(&report).contains("Model routing"));
}

#[test]
fn a_claude_policy_that_fits_passes_every_check() {
    let report = World::ready()
        .claude_ready()
        .with_policy(CLAUDE_POLICY)
        .report(None);
    assert!(report.ok, "{}", render(&report));
    let ids: Vec<_> = report.model_routing.iter().map(|c| c.id).collect();
    assert_eq!(ids, ["policy", "host", "model", "effort", "overrides"]);
    assert!(
        report.model_routing.iter().all(Check::passed),
        "{}",
        render(&report)
    );
}

#[test]
fn models_the_agent_tool_does_not_accept_are_named_per_stage() {
    let policy = r#"{"agents":{"claude":{"stages":{
        "check":{"model":"claude-opus-5-5","effort":"high"},"build":{"model":"sonnet","effort":"low"}}}}}"#;
    let report = World::ready()
        .claude_ready()
        .with_policy(policy)
        .report(None);
    let model = routing_check(&report, "model");
    assert_eq!(model.status, Status::Warning);
    assert!(
        model.message.contains("check: claude-opus-5-5"),
        "{}",
        model.message
    );
    assert!(!model.message.contains("build"), "{}", model.message);
    // The effort is fine, so it is reported separately.
    assert_eq!(routing_check(&report, "effort").status, Status::Pass);
    assert!(report.ok);
}

#[test]
fn variables_that_override_the_policy_are_found_wherever_they_are_set() {
    let ready = || World::ready().claude_ready().with_policy(CLAUDE_POLICY);
    let overrides = |world: &World| routing_check(&world.report(None), "overrides").status;

    // In the environment.
    let forced = ready()
        .with_var("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "1")
        .with_var("CLAUDE_CODE_SUBAGENT_MODEL", "haiku");
    let report = forced.report(None);
    let check = routing_check(&report, "overrides");
    assert_eq!(check.status, Status::Warning);
    assert!(
        check.message.contains("every subagent on haiku"),
        "{}",
        check.message
    );
    assert!(report.ok);

    let effort = ready().with_var("CLAUDE_CODE_EFFORT_LEVEL", "low");
    let report = effort.report(None);
    assert!(
        routing_check(&report, "overrides")
            .message
            .contains("CLAUDE_CODE_EFFORT_LEVEL=low (the environment)")
    );

    // Neither variable alone on the model side overrides the Agent tool's own choice.
    assert_eq!(
        overrides(&ready().with_var("CLAUDE_CODE_SUBAGENT_MODEL", "haiku")),
        Status::Pass
    );
    assert_eq!(
        overrides(&ready().with_var("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "1")),
        Status::Pass
    );
    assert_eq!(
        overrides(
            &ready()
                .with_var("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "0")
                .with_var("CLAUDE_CODE_SUBAGENT_MODEL", "haiku")
        ),
        Status::Pass
    );

    // In Claude Code's user settings and the project's settings files.
    let world = ready();
    let claude_dir = world.home.path().join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.json"),
        r#"{"env":{"CLAUDE_CODE_EFFORT_LEVEL":"max"}}"#,
    )
    .unwrap();
    let report = world.report(None);
    assert!(
        routing_check(&report, "overrides")
            .message
            .contains("(user settings)")
    );

    let world = ready();
    let project_dir = world.project.path().join(".claude");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(
        project_dir.join("settings.local.json"),
        r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL_FORCE":"1","CLAUDE_CODE_SUBAGENT_MODEL":"sonnet"}}"#,
    )
    .unwrap();
    let report = world.report(None);
    assert!(
        routing_check(&report, "overrides")
            .message
            .contains("(local project settings)")
    );
}

#[test]
fn a_codex_policy_reports_what_it_can_and_skips_the_rest() {
    let policy =
        r#"{"agents":{"codex":{"stages":{"build":{"model":"gpt-6.1-sol","effort":"medium"}}}}}"#;
    let report = World::ready()
        .with_agent("codex")
        .with_version("codex", "codex-cli 0.99.1")
        .with_policy(policy)
        .report(None);
    assert!(report.ok, "{}", render(&report));
    let host = routing_check(&report, "host");
    assert_eq!(host.status, Status::Skipped);
    assert!(host.message.contains("0.99.1"), "{}", host.message);
    assert_eq!(routing_check(&report, "model").status, Status::Skipped);
    assert_eq!(routing_check(&report, "overrides").status, Status::Skipped);
    assert!(
        report
            .model_routing
            .iter()
            .all(|c| c.status != Status::Fail)
    );

    let missing = World::ready().with_policy(policy).report(None);
    assert_eq!(routing_check(&missing, "host").status, Status::Warning);
}

#[test]
fn a_copilot_entry_checks_the_command_models_and_effort_and_warns_about_problems() {
    let policy = r#"{"agents":{"copilot":{"stages":{
        "check":{"model":"claude-opus-4.8","effort":"high"}}}}}"#;
    let report = World::ready()
        .claude_ready()
        .with_agent("copilot")
        .with_policy(policy)
        .report(Some("copilot"));
    let copilot = tagged(&report, "copilot");
    let ids: Vec<_> = copilot.iter().map(|c| c.id).collect();
    assert_eq!(ids, ["host", "model", "effort", "overrides"]);
    assert!(
        copilot
            .iter()
            .any(|c| c.id == "model" && c.status == Status::Pass)
    );
    assert!(copilot.iter().all(|c| !c.blocks()));
    // A missing effort is a warning, never a failure.
    let weak = r#"{"agents":{"copilot":{"stages":{
        "check":{"model":"claude-opus-4.8"},"build":{"model":"claude-sonnet-5.5","effort":"low"}}}}}"#;
    let report = World::ready()
        .claude_ready()
        .with_agent("copilot")
        .with_policy(weak)
        .report(Some("copilot"));
    assert!(report.ok, "{}", render(&report));
    let warned: Vec<_> = tagged(&report, "copilot")
        .iter()
        .filter(|c| c.status == Status::Warning)
        .map(|c| c.id)
        .collect();
    assert_eq!(warned, ["effort"]);
    // A policy that is switched off says nothing.
    let off = World::ready()
        .claude_ready()
        .with_policy(r#"{"enabled":false,"agent":"claude"}"#)
        .report(None);
    assert!(off.model_routing.is_empty());
}

#[test]
fn a_policy_file_that_stops_run_start_makes_the_report_not_ready() {
    let policy = r#"{"agents":{"claude":{"stages":{"check":{"model":"a b"}}}}}"#;
    let report = World::ready()
        .claude_ready()
        .with_policy(policy)
        .report(None);
    assert!(!report.ok);
    assert_eq!(report.model_routing.len(), 1);
    let check = routing_check(&report, "policy");
    assert_eq!(check.status, Status::Fail);
    assert!(check.message.contains("model-routing.json") && check.message.contains("Model"));
    assert!(check.fix.as_deref().unwrap().contains("enabled"));
}

#[test]
fn a_stage_without_a_model_is_a_policy_failure() {
    let policy = r#"{"agents":{"claude":{"stages":{"build":{"effort":"low"}}}}}"#;
    let report = World::ready()
        .claude_ready()
        .with_policy(policy)
        .report(None);
    assert!(!report.ok);
    let check = routing_check(&report, "policy");
    assert_eq!(check.status, Status::Fail);
    assert!(check.message.contains("model"), "{}", check.message);
}

fn provider_check<'a>(report: &'a Report, id: &str) -> Option<&'a Check> {
    report.provider.iter().find(|check| check.id == id)
}

/// A fake host tool on PATH: a shell script that exits 0 only for the listed argument lines. Tests that
/// use it run on Unix only, because the fake is a `#!/bin/sh` script.
#[cfg(unix)]
fn fake_tool(bin: &Path, name: &str, succeeds_for: &[&str]) {
    use std::os::unix::fs::PermissionsExt;
    let cases: String = succeeds_for
        .iter()
        .map(|args| format!("  \"{args}\") exit 0 ;;\n"))
        .collect();
    let file = bin.join(name);
    fs::write(
        &file,
        format!("#!/bin/sh\ncase \"$*\" in\n{cases}esac\necho secret-output\nexit 1\n"),
    )
    .unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
}

fn with_origin(world: World, url: &str) -> World {
    git(world.project.path(), &["remote", "set-url", "origin", url]);
    world
}

#[test]
fn a_github_origin_without_gh_is_a_warning_that_keeps_the_project_ready() {
    let world = with_origin(
        World::ready().claude_ready(),
        "https://github.com/me/repo.git",
    );
    let report = world.report(None);
    assert!(report.ok, "{}", render(&report));
    assert_eq!(
        provider_check(&report, "host").unwrap().status,
        Status::Pass
    );
    assert!(
        provider_check(&report, "host")
            .unwrap()
            .message
            .contains("GitHub")
    );
    assert_eq!(
        provider_check(&report, "gh").unwrap().status,
        Status::Warning
    );
    assert!(provider_check(&report, "sign-in").is_none());
    assert!(render(&report).contains("\nProvider\n"));
}

#[cfg(unix)]
#[test]
fn a_github_origin_reports_gh_and_its_sign_in() {
    let world = with_origin(World::ready(), "git@github.com:me/repo.git");
    fake_tool(world.bin.path(), "gh", &["auth status"]);
    let report = world.report(None);
    assert_eq!(provider_check(&report, "gh").unwrap().status, Status::Pass);
    assert_eq!(
        provider_check(&report, "sign-in").unwrap().status,
        Status::Pass
    );
    let not_verified = provider_check(&report, "not-verified").unwrap();
    assert_eq!(not_verified.status, Status::Skipped);
    assert!(not_verified.message.contains("token is valid"));
    assert!(not_verified.message.contains("branch policies"));

    let signed_out = with_origin(World::ready(), "git@github.com:me/repo.git");
    fake_tool(signed_out.bin.path(), "gh", &[]);
    let report = signed_out.report(None);
    let sign_in = provider_check(&report, "sign-in").unwrap();
    assert_eq!(sign_in.status, Status::Warning);
    assert!(sign_in.fix.as_deref().unwrap().contains("gh auth login"));
}

#[cfg(unix)]
const AZURE_ORIGIN: &str = "https://me@dev.azure.com/org/My%20Project/_git/repo";

#[cfg(unix)]
#[test]
fn an_azure_origin_reports_the_extension_and_the_sign_in() {
    let no_extension = with_origin(World::ready(), AZURE_ORIGIN);
    fake_tool(no_extension.bin.path(), "az", &["account show"]);
    let report = no_extension.report(None);
    assert_eq!(provider_check(&report, "az").unwrap().status, Status::Pass);
    let extension = provider_check(&report, "azure-devops-extension").unwrap();
    assert_eq!(extension.status, Status::Warning);
    assert!(
        extension
            .fix
            .as_deref()
            .unwrap()
            .contains("az extension add --name azure-devops")
    );
    // The sign-in needs no extension, so it is still reported.
    assert_eq!(
        provider_check(&report, "sign-in").unwrap().status,
        Status::Pass
    );

    let signed_out = with_origin(World::ready(), AZURE_ORIGIN);
    fake_tool(
        signed_out.bin.path(),
        "az",
        &["extension show --name azure-devops"],
    );
    let report = signed_out.report(None);
    assert_eq!(
        provider_check(&report, "azure-devops-extension")
            .unwrap()
            .status,
        Status::Pass
    );
    let sign_in = provider_check(&report, "sign-in").unwrap();
    assert_eq!(sign_in.status, Status::Warning);
    assert!(sign_in.message.contains("AZURE_DEVOPS_EXT_PAT is not set"));

    let signed_in = with_origin(World::ready(), AZURE_ORIGIN);
    fake_tool(
        signed_in.bin.path(),
        "az",
        &["extension show --name azure-devops", "account show"],
    );
    let report = signed_in.report(None);
    assert_eq!(
        provider_check(&report, "sign-in").unwrap().status,
        Status::Pass
    );
    assert!(!render(&report).contains("secret-output"));
}

#[cfg(unix)]
#[test]
fn a_personal_access_token_counts_as_signed_in_and_is_never_shown() {
    let mut world = with_origin(World::ready(), AZURE_ORIGIN);
    world
        .vars
        .insert("AZURE_DEVOPS_EXT_PAT".into(), "tok3n-value".into());
    fake_tool(
        world.bin.path(),
        "az",
        &["extension show --name azure-devops"],
    );
    let report = world.report(None);
    let sign_in = provider_check(&report, "sign-in").unwrap();
    assert_eq!(sign_in.status, Status::Pass);
    assert_eq!(sign_in.message, "AZURE_DEVOPS_EXT_PAT is set");
    assert!(!render(&report).contains("tok3n-value"));
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("tok3n-value")
    );
}
