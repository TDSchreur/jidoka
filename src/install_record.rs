//! The install record: which agents Jidoka installed skills for, written next to the model routing policy.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

/// What `jidoka install` did: the skills version (the binary version), the agents, and the Claude
/// config directories exactly as entered. Codex and Copilot roots are resolved again when needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InstallRecord {
    pub version: String,
    pub agents: Vec<String>,
    pub claude_config_dirs: Vec<String>,
}

impl InstallRecord {
    /// A record for this binary's version.
    pub(crate) fn current(agents: Vec<String>, claude_config_dirs: Vec<String>) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            agents,
            claude_config_dirs,
        }
    }
}

/// `install.json` in the same folder as the model routing policy.
pub(crate) fn path_in(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let policy = crate::routing::policy_path_in(var)?;
    Some(policy.with_file_name("install.json"))
}

/// No file is `None`; a file that cannot be read or parsed is an error naming the path.
pub(crate) fn read(path: &Path) -> io::Result<Option<InstallRecord>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(with_path(path, error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| with_path(path, io::Error::new(io::ErrorKind::InvalidData, error)))
}

pub(crate) fn write(path: &Path, record: &InstallRecord) -> io::Result<()> {
    let json = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| with_path(parent, error))?;
    }
    fs::write(path, json + "\n").map_err(|error| with_path(path, error))
}

fn with_path(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_sits_next_to_the_policy_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let var = |name: &str| match name {
            "XDG_CONFIG_HOME" | "APPDATA" => Some(base.clone().into_os_string()),
            _ => None,
        };
        let path = path_in(var).unwrap();
        assert_eq!(path, base.join("jidoka").join("install.json"));
        assert_eq!(read(&path).unwrap(), None);
        let record = InstallRecord::current(vec!["claude".into()], vec!["/a b".into()]);
        write(&path, &record).unwrap();
        assert_eq!(read(&path).unwrap(), Some(record.clone()));
        assert_eq!(record.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_broken_record_is_an_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("install.json");
        fs::write(&path, "{").unwrap();
        assert!(
            read(&path)
                .unwrap_err()
                .to_string()
                .contains("install.json")
        );
    }
}
