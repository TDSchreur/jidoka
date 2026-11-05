use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));

/// Skill directories from releases before the rename to TDS Factory.
const LEGACY_SKILLS: [&str; 2] = ["factory2", "factory2-fast"];

/// Skills maintained in this project, installed without a third-party license.
const LOCAL_SKILLS: [&str; 4] = [
    "factory",
    "factory-fast",
    "factory-balanced",
    "implement-spec-balanced",
];

pub fn install(agent: &str) -> io::Result<()> {
    let root = skills_root(agent, &home_dir()?, |name| env::var_os(name))?;
    install_to(&root)
}

/// Resolves an agent's skills folder, honoring its config directory variable.
fn skills_root(
    agent: &str,
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> io::Result<PathBuf> {
    let config_dir = |name: &str, default: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(default))
    };
    let root = match agent {
        "codex" => config_dir("CODEX_HOME", ".codex"),
        "claude" => config_dir("CLAUDE_CONFIG_DIR", ".claude"),
        "copilot" => home.join(".copilot"),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "agent must be codex, claude, or copilot",
            ))
        }
    };
    Ok(root.join("skills"))
}

fn home_dir() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = env::var_os("HOME");
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot find your home directory"))
}

fn install_to(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let mut names: Vec<&str> = BUNDLED
        .iter()
        .filter_map(|(path, _)| path.split('/').next())
        .filter(|name| *name != "licenses")
        .collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let dir = root.join(name);
        let staging = root.join(format!(".{name}.installing"));
        remove_if_exists(&staging)?;
        fs::create_dir(&staging)?;
        if let Err(error) = write_skill(name, &staging).and_then(|()| replace_dir(&staging, &dir)) {
            let _ = fs::remove_dir_all(&staging);
            return Err(io::Error::new(
                error.kind(),
                format!("cannot install skill {}: {error}", dir.display()),
            ));
        }
        println!("Installed: {}", dir.display());
    }
    for name in LEGACY_SKILLS {
        let dir = root.join(name);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("cannot remove legacy skill {}: {error}", dir.display()),
                )
            })?;
            println!("Removed: {}", dir.display());
        }
    }
    Ok(())
}

/// Replaces `dir` with `staging`, so the skill matches the bundle exactly.
fn replace_dir(staging: &Path, dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        return fs::rename(staging, dir);
    }
    let previous = dir.with_file_name(format!(
        ".{}.previous",
        dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    remove_if_exists(&previous)?;
    fs::rename(dir, &previous)?;
    if let Err(error) = fs::rename(staging, dir) {
        let _ = fs::rename(&previous, dir);
        return Err(error);
    }
    fs::remove_dir_all(previous)
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_skill(name: &str, dir: &Path) -> io::Result<()> {
    for (path, content) in BUNDLED
        .iter()
        .filter(|(path, _)| path.split('/').next() == Some(name))
    {
        let relative = path.split_once('/').unwrap().1;
        let destination = dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, content)?;
    }
    let license = if name == "security-review" {
        "licenses/awesome-copilot-LICENSE"
    } else if name == "rust-best-practices" {
        "licenses/beautiful-rust-LICENSE"
    } else if LOCAL_SKILLS.contains(&name) {
        ""
    } else {
        "licenses/mattpocock-skills-LICENSE"
    };
    if let Some((_, content)) = BUNDLED.iter().find(|(path, _)| *path == license) {
        fs::write(dir.join("LICENSE"), content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_root_honors_agent_config_directories() {
        let home = Path::new("/home/user");
        let unset = |_: &str| None;
        let set = |name: &str| Some(OsString::from(format!("/custom/{name}")));
        let empty = |_: &str| Some(OsString::new());
        let root =
            |agent, var: &dyn Fn(&str) -> Option<OsString>| skills_root(agent, home, var).unwrap();
        assert_eq!(root("claude", &unset), home.join(".claude/skills"));
        assert_eq!(root("claude", &empty), home.join(".claude/skills"));
        assert_eq!(
            root("claude", &set),
            Path::new("/custom/CLAUDE_CONFIG_DIR/skills")
        );
        assert_eq!(root("codex", &unset), home.join(".codex/skills"));
        assert_eq!(root("codex", &set), Path::new("/custom/CODEX_HOME/skills"));
        assert_eq!(root("copilot", &set), home.join(".copilot/skills"));
        assert!(skills_root("other", home, unset).is_err());
    }
    #[test]
    fn installs_bundled_skills_and_replaces_existing_ones() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join("factory")).unwrap();
        fs::write(root.join("factory/SKILL.md"), "old content").unwrap();
        fs::write(root.join("factory/stale.md"), "removed upstream").unwrap();
        for legacy in ["factory2", "factory2-fast"] {
            fs::create_dir_all(root.join(legacy)).unwrap();
            fs::write(root.join(legacy).join("SKILL.md"), "legacy").unwrap();
        }
        fs::create_dir_all(root.join("my-skill")).unwrap();
        fs::write(root.join("my-skill/SKILL.md"), "user skill").unwrap();
        install_to(root).unwrap();
        assert!(fs::read_to_string(root.join("factory/SKILL.md"))
            .unwrap()
            .contains("name: factory"));
        assert!(!root.join("factory/stale.md").exists());
        assert!(!root.join("factory2").exists());
        assert!(!root.join("factory2-fast").exists());
        assert_eq!(
            fs::read_to_string(root.join("my-skill/SKILL.md")).unwrap(),
            "user skill"
        );
        assert!(root
            .join("security-review/references/language-patterns.md")
            .is_file());
        assert!(root.join("security-review/LICENSE").is_file());
        assert!(root.join("to-spec/SKILL.md").is_file());
        assert!(root.join("factory-fast/SKILL.md").is_file());
        assert!(root.join("implement-spec-fast/SKILL.md").is_file());
        assert!(root.join("tdd/mocking.md").is_file());
        assert!(!root.join("factory-fast/LICENSE").exists());
        assert!(root.join("factory-balanced/SKILL.md").is_file());
        assert!(root.join("implement-spec-balanced/SKILL.md").is_file());
        assert!(!root.join("factory-balanced/LICENSE").exists());
        assert!(!root.join("implement-spec-balanced/LICENSE").exists());
        assert!(root.join("triage/AGENT-BRIEF.md").is_file());
        assert!(root.join("triage/LICENSE").is_file());
        assert!(root.join("rust-best-practices/SKILL.md").is_file());
        assert!(fs::read_to_string(root.join("rust-best-practices/LICENSE"))
            .unwrap()
            .contains("Morgan Linton"));
        install_to(root).unwrap();
        let leftovers: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "leftover staging dirs: {leftovers:?}");
    }
}
