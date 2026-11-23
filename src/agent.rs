//! The coding agents Jidoka works with (Claude Code, Codex and Copilot CLI): their names and where each
//! keeps its own folder, the Agent home.

use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// A coding agent. Stored and given on the command line by its lowercase name.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Agent {
    Codex,
    Claude,
    Copilot,
}

impl Agent {
    /// Every agent, in the order the install menu lists them.
    pub(crate) const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::Copilot];

    /// `codex`, `claude` or `copilot`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Copilot => "copilot",
        }
    }

    /// The agent with this name, if there is one.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.name() == name)
    }

    /// `codex, claude, or copilot`, for messages that say which names are accepted.
    pub(crate) fn list() -> String {
        let names = Self::ALL.map(Self::name);
        let (last, rest) = names.split_last().expect("there are agents");
        format!("{}, or {last}", rest.join(", "))
    }

    /// The environment variable that moves the Agent home elsewhere.
    pub(crate) fn home_var(self) -> &'static str {
        match self {
            Self::Codex => "CODEX_HOME",
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Copilot => "COPILOT_HOME",
        }
    }

    /// The Agent home's folder name under the user's home.
    fn default_folder(self) -> &'static str {
        match self {
            Self::Codex => ".codex",
            Self::Claude => ".claude",
            Self::Copilot => ".copilot",
        }
    }

    /// The Agent home, such as `~/.claude`: the agent's variable when it is set and not empty (taken as
    /// given, even when relative), otherwise the default folder under `user_home`.
    pub(crate) fn home(self, user_home: &Path, var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
        self.home_from_var(&var)
            .unwrap_or_else(|| user_home.join(self.default_folder()))
    }

    /// `home` when the user's home may be unknown: then only the agent's variable can name the Agent home.
    pub(crate) fn home_in(
        self,
        user_home: Option<&Path>,
        var: impl Fn(&str) -> Option<OsString>,
    ) -> Option<PathBuf> {
        match user_home {
            Some(user_home) => Some(self.home(user_home, var)),
            None => self.home_from_var(&var),
        }
    }

    fn home_from_var(self, var: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        var(self.home_var())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_parses_from_its_name_and_others_do_not() {
        for agent in Agent::ALL {
            assert_eq!(Agent::parse(agent.name()), Some(agent));
            let stored = serde_json::to_value(agent).unwrap();
            assert_eq!(stored, agent.name());
            assert_eq!(serde_json::from_value::<Agent>(stored).unwrap(), agent);
        }
        assert_eq!(Agent::parse("Claude"), None);
        assert_eq!(Agent::parse("gemini"), None);
        assert_eq!(Agent::list(), "codex, claude, or copilot");
    }

    #[test]
    fn the_home_is_the_agents_variable_or_its_folder_in_the_user_home() {
        let user = Path::new("/home/user");
        let unset = |_: &str| None;
        let empty = |_: &str| Some(OsString::new());
        let set = |name: &str| Some(OsString::from(format!("/custom/{name}")));
        let relative = |_: &str| Some(OsString::from("work"));
        for (agent, folder) in [
            (Agent::Codex, ".codex"),
            (Agent::Claude, ".claude"),
            (Agent::Copilot, ".copilot"),
        ] {
            assert_eq!(agent.home(user, unset), user.join(folder));
            assert_eq!(agent.home(user, empty), user.join(folder));
            assert_eq!(
                agent.home(user, set),
                Path::new("/custom").join(agent.home_var())
            );
            assert_eq!(agent.home(user, relative), Path::new("work"));
        }
    }

    #[test]
    fn without_a_user_home_only_the_variable_names_the_home() {
        let set = |name: &str| (name == "CODEX_HOME").then(|| OsString::from("/custom/codex"));
        assert_eq!(
            Agent::Codex.home_in(None, set),
            Some(PathBuf::from("/custom/codex"))
        );
        assert_eq!(Agent::Claude.home_in(None, set), None);
        assert_eq!(Agent::Claude.home_in(None, |_| Some(OsString::new())), None);
        assert_eq!(
            Agent::Claude.home_in(Some(Path::new("/home/user")), set),
            Some(PathBuf::from("/home/user/.claude"))
        );
    }
}
