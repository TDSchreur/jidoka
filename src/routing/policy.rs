//! The model-routing policy file: reading it, what it means for a run that starts, and the starter
//! `factory doctor --create-model-routing` writes.
//!
//! The file has two forms. The single-agent form (`agent`, `roles`, `tasks`) is the original one and keeps
//! working. The multi-agent form has an `agents` object with an entry per agent (`claude`, `codex`), each
//! with its own `roles`, `tasks` and an optional `enabled`. Mixing the two forms is an error.
//!
//! A run starts in one agent. For a multi-agent file the caller must say which (`--agent`), and the run
//! records only that agent's policy: another agent's policy is never used instead.

use super::{ModelRouting, Role, RoleChoice, RoutingAgent, Task};
use serde::{de, Deserialize, Deserializer};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt, fs,
    io::{self, Read, Write},
    marker::PhantomData,
    path::{Path, PathBuf},
};

/// The policy file is user-owned and could be anything: larger files are not read.
const MAX_POLICY_BYTES: u64 = 1 << 20;

pub(crate) const COPILOT_NOTICE: &str =
    "Model routing is not supported on Copilot CLI: every stage uses the session's model.";

/// The agent a run starts in. Copilot CLI is a valid host, but nothing is routed for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Host {
    Claude,
    Codex,
    Copilot,
}

impl Host {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "copilot" => Some(Self::Copilot),
            _ => None,
        }
    }

    pub(crate) fn routing_agent(self) -> Option<RoutingAgent> {
        match self {
            Self::Claude => Some(RoutingAgent::Claude),
            Self::Codex => Some(RoutingAgent::Codex),
            Self::Copilot => None,
        }
    }
}

impl From<RoutingAgent> for Host {
    fn from(agent: RoutingAgent) -> Self {
        match agent {
            RoutingAgent::Claude => Self::Claude,
            RoutingAgent::Codex => Self::Codex,
        }
    }
}

/// The options of `factory run start <title>`: only `--agent <name>`.
pub(crate) fn parse_start_options(options: &[&str]) -> Result<Option<Host>, String> {
    match options {
        [] => Ok(None),
        ["--agent"] => Err("--agent needs codex, claude, or copilot.".into()),
        ["--agent", name] => Host::parse(name)
            .map(Some)
            .ok_or_else(|| format!("Unknown agent {name:?}; use codex, claude, or copilot.")),
        ["--agent", _, extra, ..] | [extra, ..] => Err(format!("Unexpected option {extra:?}.")),
    }
}

/// One agent's roles and the task mapping, as written in the file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct AgentPolicy {
    /// A disabled entry is not checked and routes nothing.
    pub(crate) enabled: bool,
    pub(crate) roles: BTreeMap<Role, RoleChoice>,
    pub(crate) tasks: BTreeMap<Task, Role>,
}

impl AgentPolicy {
    /// What a run records for this agent: the roles and the task mapping, with no overrides yet.
    pub(crate) fn selection(&self, agent: RoutingAgent) -> ModelRouting {
        ModelRouting {
            agent,
            roles: self.roles.clone(),
            tasks: self.tasks.clone(),
            overrides: BTreeMap::new(),
        }
    }
}

/// What a policy file says.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Policy {
    /// No file, or switched off at the top. Nothing in a switched-off file is checked.
    Off,
    /// The single-agent form for Copilot CLI: nothing is routed.
    CopilotOnly,
    /// The single-agent form.
    Single {
        agent: RoutingAgent,
        policy: AgentPolicy,
    },
    /// The `agents` form. At least one entry is present.
    Multi {
        claude: Option<AgentPolicy>,
        codex: Option<AgentPolicy>,
    },
}

/// What a policy means for a new run: the selection to record, if any, and a notice for the user.
#[derive(Default, PartialEq, Eq, Debug)]
pub(crate) struct Resolution {
    pub(crate) selection: Option<ModelRouting>,
    pub(crate) notice: Option<String>,
}

impl Resolution {
    fn selection(selection: ModelRouting) -> Self {
        Self {
            selection: Some(selection),
            notice: None,
        }
    }

    /// The result for a run in `host`, from `Policy::coverage`. A switched-off entry says nothing.
    fn resolved(policy: &Policy, host: Host) -> Self {
        match policy.coverage(host) {
            None | Some(Coverage::NotRouted(NotRouted::SwitchedOff)) => Self::default(),
            Some(Coverage::Routed(selection)) => Self::selection(selection),
            Some(Coverage::NotRouted(why)) => Self {
                selection: None,
                notice: Some(why.text(host)),
            },
        }
    }
}

/// Why a run in some agent is not routed by a policy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum NotRouted {
    /// Copilot CLI is not supported.
    Copilot,
    /// A single-agent policy written for this other agent.
    OtherAgent(RoutingAgent),
    /// The `agents` form has no entry for the agent.
    NoEntry,
    /// The agent's entry has `enabled: false`.
    SwitchedOff,
}

impl NotRouted {
    /// What to tell the user about a run in `host`.
    pub(crate) fn text(self, host: Host) -> String {
        match self {
            Self::Copilot => COPILOT_NOTICE.to_string(),
            Self::OtherAgent(agent) => format!(
                "The model-routing policy is for {}, so this {} run is not routed: every stage uses the session's model. To route {} too, put both agents under \"agents\" (see the README).",
                agent.name(),
                host.name(),
                host.name()
            ),
            Self::NoEntry => format!(
                "The model-routing policy has no entry for {0}, so this run is not routed: every stage uses the session's model. Add \"agents\".\"{0}\" to route it.",
                host.name()
            ),
            Self::SwitchedOff => format!(
                "The {} entry of the model-routing policy is switched off, so this run is not routed: every stage uses the session's model.",
                host.name()
            ),
        }
    }
}

/// What a policy does for a run in one agent.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Coverage {
    Routed(ModelRouting),
    NotRouted(NotRouted),
}

impl Policy {
    /// What the policy does for a run in `host`: the one place that decides which agent's part applies and
    /// why none does, for `factory run start` and for `factory doctor` alike. `None` when the policy is off.
    pub(crate) fn coverage(&self, host: Host) -> Option<Coverage> {
        let agent = match (self, host.routing_agent()) {
            (Self::Off, _) => return None,
            (Self::CopilotOnly, _) | (_, None) => {
                return Some(Coverage::NotRouted(NotRouted::Copilot))
            }
            (_, Some(agent)) => agent,
        };
        Some(match self {
            Self::Single {
                agent: policy_agent,
                policy,
            } if *policy_agent == agent => Coverage::Routed(policy.selection(agent)),
            Self::Single {
                agent: policy_agent,
                ..
            } => Coverage::NotRouted(NotRouted::OtherAgent(*policy_agent)),
            Self::Multi { .. } => match self.entry(agent) {
                None => Coverage::NotRouted(NotRouted::NoEntry),
                Some(entry) if !entry.enabled => Coverage::NotRouted(NotRouted::SwitchedOff),
                Some(entry) => Coverage::Routed(entry.selection(agent)),
            },
            Self::Off | Self::CopilotOnly => unreachable!("handled above"),
        })
    }

    /// The policy for a run that starts in `host`. `None` means the caller did not say, which only the
    /// single-agent form allows (it always has, and then the agent named in the file is recorded).
    pub(crate) fn resolve(&self, host: Option<Host>) -> Result<Resolution, String> {
        let Some(host) = host else {
            return match self {
                Self::Off | Self::CopilotOnly => Ok(Resolution::resolved(self, Host::Copilot)),
                Self::Single { agent, policy } => {
                    Ok(Resolution::selection(policy.selection(*agent)))
                }
                Self::Multi { claude, codex } => {
                    if [claude, codex]
                        .iter()
                        .all(|e| e.as_ref().is_none_or(|e| !e.enabled))
                    {
                        return Ok(Resolution::default());
                    }
                    Err(format!(
                        "the policy configures several agents ({}), so `factory run start` must be told which agent the run is in: add `--agent claude` or `--agent codex`",
                        self.configured().join(", ")
                    ))
                }
            };
        };
        Ok(Resolution::resolved(self, host))
    }

    /// The entry for `agent` in the multi-agent form.
    pub(crate) fn entry(&self, agent: RoutingAgent) -> Option<&AgentPolicy> {
        match (self, agent) {
            (Self::Multi { claude, .. }, RoutingAgent::Claude) => claude.as_ref(),
            (Self::Multi { codex, .. }, RoutingAgent::Codex) => codex.as_ref(),
            _ => None,
        }
    }

    /// The agents that have an entry in the multi-agent form, in a fixed order.
    pub(crate) fn configured(&self) -> Vec<&'static str> {
        [RoutingAgent::Claude, RoutingAgent::Codex]
            .into_iter()
            .filter(|agent| self.entry(*agent).is_some())
            .map(RoutingAgent::name)
            .collect()
    }
}

fn enabled_by_default() -> bool {
    true
}

/// A map that refuses a key it has already seen, which `BTreeMap` would quietly overwrite.
struct Unique<K, V>(BTreeMap<K, V>);

impl<'de, K, V> Deserialize<'de> for Unique<K, V>
where
    K: Deserialize<'de> + Ord + fmt::Debug,
    V: Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit<K, V>(PhantomData<(K, V)>);
        impl<'de, K, V> de::Visitor<'de> for Visit<K, V>
        where
            K: Deserialize<'de> + Ord + fmt::Debug,
            V: Deserialize<'de>,
        {
            type Value = Unique<K, V>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<K, V>()? {
                    let shown = format!("{key:?}").to_lowercase();
                    if out.insert(key, value).is_some() {
                        return Err(de::Error::custom(format!("duplicate entry `{shown}`")));
                    }
                }
                Ok(Unique(out))
            }
        }
        deserializer.deserialize_map(Visit(PhantomData))
    }
}

/// One entry of `agents`, as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    roles: Unique<Role, RoleChoice>,
    tasks: Unique<Task, Role>,
}

/// The `agents` object, as written. A repeated agent is a duplicate-field error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Agents {
    claude: Option<Entry>,
    codex: Option<Entry>,
}

/// The policy file as the user writes it, before the two forms are told apart.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// Read from the raw text first (see `read`); accepted here so the file may spell it out.
    #[serde(default = "enabled_by_default")]
    #[allow(dead_code)]
    enabled: bool,
    agent: Option<RoutingAgent>,
    roles: Option<Unique<Role, RoleChoice>>,
    tasks: Option<Unique<Task, Role>>,
    agents: Option<Agents>,
}

impl From<Entry> for AgentPolicy {
    fn from(entry: Entry) -> Self {
        Self {
            enabled: entry.enabled,
            roles: entry.roles.0,
            tasks: entry.tasks.0,
        }
    }
}

impl File {
    fn into_policy(self) -> Result<Policy, String> {
        match (self.agents, self.agent, self.roles, self.tasks) {
            (Some(agents), None, None, None) => agents.into_policy(),
            (Some(_), _, _, _) => Err(
                "it mixes `agent`, `roles` or `tasks` with `agents`; use one form: either those three fields for one agent, or `agents` with an entry per agent".into(),
            ),
            (None, Some(agent), Some(roles), Some(tasks)) => {
                let policy = AgentPolicy {
                    enabled: true,
                    roles: roles.0,
                    tasks: tasks.0,
                };
                policy.selection(agent).validate()?;
                Ok(Policy::Single { agent, policy })
            }
            (None, None, _, _) => Err(
                "missing field `agent` (or use `agents` with an entry per agent)".into(),
            ),
            (None, Some(_), None, _) => Err("missing field `roles`".into()),
            (None, Some(_), Some(_), None) => Err("missing field `tasks`".into()),
        }
    }
}

impl Agents {
    fn into_policy(self) -> Result<Policy, String> {
        if self.claude.is_none() && self.codex.is_none() {
            return Err("`agents` has no entry: add `claude`, `codex` or both".into());
        }
        let checked = |name: &str, agent, entry: Option<Entry>| {
            entry
                .map(AgentPolicy::from)
                .map(|policy| {
                    // A switched-off entry is not checked, so it can be half edited meanwhile.
                    if policy.enabled {
                        policy
                            .selection(agent)
                            .validate()
                            .map_err(|problem| format!("agents.{name}: {problem}"))?;
                    }
                    Ok::<_, String>(policy)
                })
                .transpose()
        };
        Ok(Policy::Multi {
            claude: checked("claude", RoutingAgent::Claude, self.claude)?,
            codex: checked("codex", RoutingAgent::Codex, self.codex)?,
        })
    }
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

/// Reads and checks the policy. No file is `Policy::Off`. A file that cannot be read or is wrong is an
/// error with its path, so routing never quietly does less than the user asked for.
pub(crate) fn read(path: &Path) -> Result<Policy, String> {
    let fail = |problem: String| format!("{}: {problem}", path.display());
    let cannot = |error: io::Error| fail(format!("cannot read the model-routing policy: {error}"));
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(fail(
                "the model-routing policy is not a regular file".into(),
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Policy::Off),
        Err(error) => return Err(cannot(error)),
    }
    // Bounded while reading, not only by the size seen before: the file can grow in between. The kind is
    // checked again on the open file, so a swap after the check above is still caught.
    let mut text = String::new();
    let mut file = fs::File::open(path).map_err(cannot)?;
    if !file.metadata().map_err(cannot)?.is_file() {
        return Err(fail(
            "the model-routing policy is not a regular file".into(),
        ));
    }
    (&mut file)
        .take(MAX_POLICY_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(cannot)?;
    if text.len() as u64 > MAX_POLICY_BYTES {
        return Err(fail("the model-routing policy is too large".into()));
    }
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        fail(format!(
            "the model-routing policy is not valid JSON: {error}"
        ))
    })?;
    // A switched-off policy is not checked, so a file can be half edited while it is off.
    if value.get("enabled") == Some(&Value::Bool(false)) {
        return Ok(Policy::Off);
    }
    // Copilot CLI records no selection, so the rest of its single-agent file is not read.
    if value.get("agents").is_none() && value.get("agent") == Some(&Value::String("copilot".into()))
    {
        return Ok(Policy::CopilotOnly);
    }
    if value.pointer("/agents/copilot").is_some() {
        return Err(fail(
            "invalid model-routing policy: `agents.copilot`: model routing is not supported on Copilot CLI, so there is nothing to configure for it; remove the entry".into(),
        ));
    }
    // Read again from the text: the typed form refuses repeated keys, which a parsed value would not show.
    let file: File = serde_json::from_str(&text)
        .map_err(|error| fail(format!("invalid model-routing policy: {error}")))?;
    file.into_policy()
        .map_err(|problem| fail(format!("invalid model-routing policy: {problem}")))
}

/// The policy for a run that is starting in `host`, which may be unknown only for the single-agent form.
pub(crate) fn load(path: &Path, host: Option<Host>) -> Result<Resolution, String> {
    // Nothing is routed on Copilot CLI, so a mistake in the file cannot matter to such a run.
    if host == Some(Host::Copilot) {
        return Ok(read(path).map_or_else(
            |_| Resolution::default(),
            |policy| Resolution::resolved(&policy, Host::Copilot),
        ));
    }
    read(path)?
        .resolve(host)
        .map_err(|problem| format!("{}: {problem}", path.display()))
}

/// The three roles of a starter entry. Only the `check` task is routed so far, so only it is mapped.
fn starter_entry(agent: RoutingAgent) -> String {
    let ((strong, strong_effort), (coding, coding_effort), (routine, routine_effort)) = match agent
    {
        RoutingAgent::Claude => (("opus", "high"), ("sonnet", "medium"), ("haiku", "low")),
        RoutingAgent::Codex => (
            ("gpt-6-astra", "high"),
            ("gpt-6.1-sol", "medium"),
            ("gpt-6-luna", "low"),
        ),
    };
    format!(
        r#"    "{}": {{
      "roles": {{
        "strong": {{ "model": "{strong}", "effort": "{strong_effort}" }},
        "coding": {{ "model": "{coding}", "effort": "{coding_effort}" }},
        "routine": {{ "model": "{routine}", "effort": "{routine_effort}" }}
      }},
      "tasks": {{ "check": "strong" }}
    }}"#,
        agent.name()
    )
}

/// A starting policy in the multi-agent form, written by `factory doctor --create-model-routing`: an entry
/// with all three roles for each agent in `agents`, routing only `check`, the one task that uses routing.
fn starter(agents: &[RoutingAgent]) -> String {
    let entries: Vec<String> = agents.iter().map(|agent| starter_entry(*agent)).collect();
    format!(
        "{{\n  \"enabled\": true,\n  \"agents\": {{\n{}\n  }}\n}}\n",
        entries.join(",\n")
    )
}

/// Writes the starter for `agents` to `path`, creating its folder. Whatever is already at `path`, even a
/// link that points nowhere, is left alone and `false` is returned.
pub(crate) fn create_starter(path: &Path, agents: &[RoutingAgent]) -> io::Result<bool> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            // A half-written starter would look like the user's own file and block every run start.
            if let Err(error) = file.write_all(starter(agents).as_bytes()) {
                drop(file);
                return Err(match fs::remove_file(path) {
                    Ok(()) => error,
                    Err(_) => io::Error::new(
                        error.kind(),
                        format!(
                            "{error}; remove the partly written {} by hand",
                            path.display()
                        ),
                    ),
                });
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
