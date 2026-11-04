use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundled_skills.rs"));

pub fn install(agent: &str) -> io::Result<()> {
    let home = home_dir()?;
    let root = match agent {
        "codex" => env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("skills"),
        "claude" => home.join(".claude/skills"),
        "copilot" => home.join(".copilot/skills"),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "agent must be codex, claude, or copilot",
            ))
        }
    };
    install_to(&root)
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
