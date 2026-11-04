use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));

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
        match fs::create_dir(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                println!("Skipped existing skill: {}", dir.display());
                continue;
            }
            Err(e) => return Err(e),
        }
        if let Err(error) = write_skill(name, &dir) {
            let _ = fs::remove_dir_all(&dir);
            return Err(error);
        }
        println!("Installed: {}", dir.display());
    }
    Ok(())
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
    } else if name != "factory2" && name != "factory2-fast" {
        "licenses/mattpocock-skills-LICENSE"
    } else {
        ""
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
    fn installs_bundled_skills_without_overwriting_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join("factory2")).unwrap();
        fs::write(root.join("factory2/SKILL.md"), "user content").unwrap();
        install_to(root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("factory2/SKILL.md")).unwrap(),
            "user content"
        );
        assert!(root
            .join("security-review/references/language-patterns.md")
            .is_file());
        assert!(root.join("security-review/LICENSE").is_file());
        assert!(root.join("to-spec/SKILL.md").is_file());
        assert!(root.join("factory2-fast/SKILL.md").is_file());
        assert!(root.join("implement-spec-fast/SKILL.md").is_file());
        assert!(!root.join("factory2-fast/LICENSE").exists());
        assert!(root.join("triage/AGENT-BRIEF.md").is_file());
        assert!(root.join("triage/LICENSE").is_file());
        assert!(root.join("rust-best-practices/SKILL.md").is_file());
        assert!(fs::read_to_string(root.join("rust-best-practices/LICENSE"))
            .unwrap()
            .contains("Morgan Linton"));
        install_to(root).unwrap();
    }
}
