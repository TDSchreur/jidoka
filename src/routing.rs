//! Model routing: which model and effort each kind of task should use, recorded on a run when it starts.
//! "Routing" alone already means workflow selection in this repository, so this is always model routing.
//! The record holds only the roles and the task mapping; no stage reads it yet.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// The coding agent a selection is for. Copilot CLI is not here: it records no selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RoutingAgent {
    Codex,
    Claude,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    Strong,
    Coding,
    Routine,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Task {
    Build,
    Check,
    Finish,
}

/// A role's model and effort. Both are free text: the factory keeps no list of valid values.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoleChoice {
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
}

/// The selection recorded on a run.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelRouting {
    pub(crate) agent: RoutingAgent,
    pub(crate) roles: BTreeMap<Role, RoleChoice>,
    /// The policy's mapping of tasks to roles, as it was when the run started.
    pub(crate) tasks: BTreeMap<Task, Role>,
    /// Roles chosen for this run by hand; they win over `tasks` and leave it untouched.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) overrides: BTreeMap<Task, Role>,
}

/// Whether an observed model id is the model that was requested. A requested alias is a whole part of the
/// id (`sonnet` in `claude-sonnet-5-5`); a version is not cut (`gpt-6` is not in `gpt-6.1-sol`).
pub(crate) fn model_matches(requested: &str, observed: &str) -> bool {
    let (requested, observed) = (
        requested.to_ascii_lowercase(),
        observed.to_ascii_lowercase(),
    );
    let boundary = |c: Option<char>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '.'));
    !requested.is_empty()
        && observed.match_indices(&requested).any(|(start, _)| {
            boundary(observed[..start].chars().next_back())
                && boundary(observed[start + requested.len()..].chars().next())
        })
}

/// A starting policy for `agent`, written by `factory doctor --create-model-routing`. It defines all three
/// roles so they can be extended, and routes only `check`, the one task that uses routing so far.
fn starter(agent: RoutingAgent) -> &'static str {
    match agent {
        RoutingAgent::Claude => {
            r#"{
  "enabled": true,
  "agent": "claude",
  "roles": {
    "strong": { "model": "opus", "effort": "high" },
    "coding": { "model": "sonnet", "effort": "medium" },
    "routine": { "model": "haiku", "effort": "low" }
  },
  "tasks": { "check": "strong" }
}
"#
        }
        RoutingAgent::Codex => {
            r#"{
  "enabled": true,
  "agent": "codex",
  "roles": {
    "strong": { "model": "gpt-6-astra", "effort": "high" },
    "coding": { "model": "gpt-6.1-sol", "effort": "medium" },
    "routine": { "model": "gpt-6-luna", "effort": "low" }
  },
  "tasks": { "check": "strong" }
}
"#
        }
    }
}

/// Writes the starter policy to `path`, creating its folder. Whatever is already at `path`, even a link
/// that points nowhere, is left alone and `false` is returned.
pub(crate) fn create_starter(path: &Path, agent: RoutingAgent) -> io::Result<bool> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(starter(agent).as_bytes())?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

/// Longest model or effort text. They are shown in a terminal and handed to agents, so they stay short.
const MAX_TEXT: usize = 64;

fn check_text(name: &str, value: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-');
    if value.is_empty() || value.len() > MAX_TEXT || !value.chars().all(allowed) {
        return Err(format!(
            "{name} must be 1 to {MAX_TEXT} characters from letters, digits and . _ : / -"
        ));
    }
    Ok(())
}

impl ModelRouting {
    /// The role that handles `task`: this run's override if it has one, otherwise the policy's mapping.
    pub(crate) fn role_for(&self, task: Task) -> Option<Role> {
        self.overrides
            .get(&task)
            .or_else(|| self.tasks.get(&task))
            .copied()
    }

    /// The rules every selection follows, whether the CLI resolved it or the API received it.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for choice in self.roles.values() {
            check_text("Model", &choice.model)?;
            if let Some(effort) = &choice.effort {
                check_text("Effort", effort)?;
            }
        }
        for (task, role) in self.tasks.iter().chain(&self.overrides) {
            if !self.roles.contains_key(role) {
                return Err(format!(
                    "The {} task uses the {} role, which is not defined under roles",
                    task.name(),
                    role.name()
                ));
            }
        }
        Ok(())
    }
}

impl RoutingAgent {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

impl Role {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Coding => "coding",
            Self::Routine => "routine",
        }
    }
}

impl Task {
    pub(crate) const ALL: [Task; 3] = [Task::Build, Task::Check, Task::Finish];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Check => "check",
            Self::Finish => "finish",
        }
    }
}

/// The policy file is user-owned and could be anything: larger files are not read.
const MAX_POLICY_BYTES: u64 = 1 << 20;

fn enabled_by_default() -> bool {
    true
}

/// The policy file as the user writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    /// Accepted so the file may spell it out; a switched-off policy never gets this far (see `load`).
    #[serde(default = "enabled_by_default", rename = "enabled")]
    _enabled: bool,
    agent: RoutingAgent,
    roles: BTreeMap<Role, RoleChoice>,
    tasks: BTreeMap<Task, Role>,
}

/// What a policy file means for a new run: the selection to record, if any, and a notice for the user.
#[derive(Default, PartialEq, Eq, Debug)]
pub(crate) struct Resolution {
    pub(crate) selection: Option<ModelRouting>,
    pub(crate) notice: Option<String>,
}

/// Where the user-level policy lives: `$XDG_CONFIG_HOME/tds-factory/` or `~/.config/tds-factory/`,
/// and `%APPDATA%\tds-factory\` on Windows.
pub(crate) fn policy_path_in(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    // A relative folder would put the file wherever the command happens to run, so, as the XDG spec says,
    // it is ignored.
    let set = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let base = if cfg!(windows) {
        set("APPDATA")?
    } else {
        set("XDG_CONFIG_HOME").or_else(|| set("HOME").map(|home| home.join(".config")))?
    };
    Some(base.join("tds-factory").join("model-routing.json"))
}

/// Reads the policy for a run that is starting. No file means no routing. A file that cannot be read or
/// is wrong stops the start with its path, so routing never quietly does less than the user asked for.
pub(crate) fn load(path: &Path) -> Result<Resolution, String> {
    let fail = |problem: String| format!("{}: {problem}", path.display());
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Resolution::default()),
        Err(error) => {
            return Err(fail(format!(
                "cannot read the model-routing policy: {error}"
            )))
        }
    };
    if size > MAX_POLICY_BYTES {
        return Err(fail("the model-routing policy is too large".into()));
    }
    let text = fs::read_to_string(path)
        .map_err(|error| fail(format!("cannot read the model-routing policy: {error}")))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        fail(format!(
            "the model-routing policy is not valid JSON: {error}"
        ))
    })?;
    // A switched-off policy is not checked, so a file can be half edited while it is off.
    if value.get("enabled") == Some(&serde_json::Value::Bool(false)) {
        return Ok(Resolution::default());
    }
    // Copilot CLI records no selection, so the rest of its file is not read.
    if value.get("agent") == Some(&serde_json::Value::String("copilot".into())) {
        return Ok(Resolution {
            selection: None,
            notice: Some(
                "Model routing is not supported on Copilot CLI: every stage uses the session's model."
                    .into(),
            ),
        });
    }
    let policy: Policy = serde_json::from_value(value)
        .map_err(|error| fail(format!("invalid model-routing policy: {error}")))?;
    let selection = ModelRouting {
        agent: policy.agent,
        roles: policy.roles,
        tasks: policy.tasks,
        overrides: BTreeMap::new(),
    };
    selection
        .validate()
        .map_err(|problem| fail(format!("invalid model-routing policy: {problem}")))?;
    Ok(Resolution {
        selection: Some(selection),
        notice: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn load_text(text: &str) -> Result<Resolution, String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-routing.json");
        fs::write(&path, text).unwrap();
        load(&path)
    }

    const CLAUDE: &str = r#"{
        "enabled": true,
        "agent": "claude",
        "roles": {
            "strong": {"model": "opus", "effort": "high"},
            "coding": {"model": "sonnet", "effort": "medium"},
            "routine": {"model": "haiku", "effort": "low"}
        },
        "tasks": {"build": "coding", "check": "strong", "finish": "routine"}
    }"#;

    #[test]
    fn each_starter_policy_loads_and_routes_check_to_the_strong_role() {
        let dir = tempfile::tempdir().unwrap();
        for agent in [RoutingAgent::Claude, RoutingAgent::Codex] {
            let path = dir.path().join(format!("{}.json", agent.name()));
            assert!(create_starter(&path, agent).unwrap());
            let selection = load(&path).unwrap().selection.expect("the starter routes");
            assert_eq!(selection.agent, agent);
            assert_eq!(
                selection.tasks.keys().copied().collect::<Vec<_>>(),
                [Task::Check]
            );
            assert_eq!(selection.role_for(Task::Check), Some(Role::Strong));
            // All three roles are there to extend, each with a model and an effort.
            for role in [Role::Strong, Role::Coding, Role::Routine] {
                let choice = &selection.roles[&role];
                assert!(
                    !choice.model.is_empty() && choice.effort.is_some(),
                    "{agent:?} {role:?}"
                );
            }
        }
    }

    #[test]
    fn a_starter_is_created_with_its_folder_and_never_replaces_anything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tds-factory/model-routing.json");
        assert!(create_starter(&path, RoutingAgent::Claude).unwrap());
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("\"agent\": \"claude\""));
        // A second call, even for another agent, leaves the file alone.
        fs::write(&path, "my own policy").unwrap();
        assert!(!create_starter(&path, RoutingAgent::Codex).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "my own policy");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_policy_path_is_left_alone_even_when_it_points_nowhere() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.json");
        let path = dir.path().join("model-routing.json");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(!create_starter(&path, RoutingAgent::Claude).unwrap());
        assert!(
            !target.exists(),
            "the link must not be followed to create a file"
        );
    }

    #[test]
    fn a_requested_model_matches_an_observed_id_by_whole_parts() {
        for (requested, observed) in [
            ("sonnet", "claude-sonnet-5-5"),
            ("opus", "claude-opus-5-5"),
            ("OPUS", "claude-opus-4-8"),
            ("haiku", "claude-haiku-5-5"),
            ("gpt-6.1-sol", "gpt-6.1-sol"),
            ("gpt-6-astra", "gpt-6-astra"),
        ] {
            assert!(
                model_matches(requested, observed),
                "{requested} / {observed}"
            );
        }
        for (requested, observed) in [
            ("opus", "claude-sonnet-5-5"),
            ("gpt-6", "gpt-6.1-sol"),
            ("son", "claude-sonnet-5-5"),
            ("", "claude-sonnet-5-5"),
        ] {
            assert!(
                !model_matches(requested, observed),
                "{requested} / {observed}"
            );
        }
    }

    #[test]
    fn without_a_policy_file_nothing_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let resolution = load(&dir.path().join("model-routing.json")).unwrap();
        assert_eq!(resolution, Resolution::default());
    }

    #[test]
    fn a_claude_policy_becomes_the_recorded_selection() {
        let selection = load_text(CLAUDE).unwrap().selection.unwrap();
        assert_eq!(selection.agent, RoutingAgent::Claude);
        assert_eq!(selection.roles[&Role::Strong].model, "opus");
        assert_eq!(
            selection.roles[&Role::Routine].effort.as_deref(),
            Some("low")
        );
        assert_eq!(selection.tasks[&Task::Check], Role::Strong);
        assert_eq!(selection.tasks[&Task::Finish], Role::Routine);
    }

    #[test]
    fn a_codex_policy_keeps_its_own_models_and_an_omitted_effort_and_task() {
        let text = r#"{"agent":"codex","roles":{"coding":{"model":"gpt-6.1-sol"}},"tasks":{"build":"coding"}}"#;
        let selection = load_text(text).unwrap().selection.unwrap();
        assert_eq!(selection.agent, RoutingAgent::Codex);
        assert_eq!(selection.roles[&Role::Coding].effort, None);
        assert_eq!(selection.tasks.get(&Task::Check), None);
    }

    #[test]
    fn a_disabled_policy_records_nothing_even_if_it_has_other_mistakes() {
        let text = CLAUDE.replace("\"enabled\": true", "\"enabled\": false");
        assert_eq!(load_text(&text).unwrap(), Resolution::default());
        let broken = r#"{"enabled":false,"agent":"claude","roles":{},"tasks":{"check":"strong"}}"#;
        assert_eq!(load_text(broken).unwrap(), Resolution::default());
    }

    #[test]
    fn a_copilot_policy_records_nothing_and_says_why() {
        let text = r#"{"agent":"copilot","roles":{"coding":{"model":"claude-sonnet-5.5"}},"tasks":{"build":"coding"}}"#;
        let resolution = load_text(text).unwrap();
        assert_eq!(resolution.selection, None);
        let notice = resolution.notice.unwrap();
        assert!(
            notice.contains("Copilot") && notice.contains("session"),
            "{notice}"
        );
    }

    #[test]
    fn a_bare_copilot_policy_gets_the_notice_instead_of_a_parse_error() {
        let resolution = load_text(r#"{"agent":"copilot"}"#).unwrap();
        assert_eq!(resolution.selection, None);
        assert!(resolution.notice.unwrap().contains("Copilot"));
        // Switched off, it says nothing.
        assert_eq!(
            load_text(r#"{"agent":"copilot","enabled":false}"#).unwrap(),
            Resolution::default()
        );
    }

    #[test]
    fn mistakes_in_the_file_are_errors_that_name_the_file_and_the_problem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-routing.json");
        for (text, problem) in [
            ("not json", "model-routing.json"),
            (
                r#"{"agent":"claude","roles":{},"tasks":{},"extra":1}"#,
                "extra",
            ),
            (r#"{"agent":"gemini","roles":{},"tasks":{}}"#, "gemini"),
            (r#"{"roles":{},"tasks":{}}"#, "agent"),
            (
                r#"{"agent":"claude","roles":{"coding":{"model":"sonnet"}},"tasks":{"check":"strong"}}"#,
                "strong",
            ),
            (
                r#"{"agent":"claude","roles":{"coding":{"model":"a b"}},"tasks":{}}"#,
                "Model",
            ),
        ] {
            fs::write(&path, text).unwrap();
            let error = load(&path).unwrap_err();
            assert!(error.contains(&path.display().to_string()), "{error}");
            assert!(error.contains(problem), "{text}: {error}");
        }
    }

    #[test]
    fn an_unreadable_or_oversized_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-routing.json");
        fs::create_dir(&path).unwrap();
        assert!(load(&path).is_err());
        let big = dir.path().join("big.json");
        fs::write(&big, " ".repeat(1 << 20) + "{}").unwrap();
        assert!(load(&big).unwrap_err().contains("too large"));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_policy_lives_in_the_factorys_config_directory() {
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
        assert_eq!(path(&[]), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn a_relative_config_folder_is_ignored_as_the_xdg_spec_says() {
        let path = |vars: &'static [(&'static str, &'static str)]| {
            policy_path_in(move |name| {
                vars.iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).into())
            })
        };
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
        assert_eq!(path(&[("HOME", ".")]), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_policy_lives_in_the_factorys_config_directory() {
        let path = policy_path_in(|name| {
            (name == "APPDATA").then(|| r"C:\Users\u\AppData\Roaming".into())
        });
        assert_eq!(
            path,
            Some(
                PathBuf::from(r"C:\Users\u\AppData\Roaming")
                    .join("tds-factory")
                    .join("model-routing.json")
            )
        );
    }
}
