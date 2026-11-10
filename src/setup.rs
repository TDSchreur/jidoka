//! Startup check: adopts skills installed before the install record existed, or hints at `jidoka install`.
use crate::install_record::{self, InstallRecord};
use std::{ffi::OsString, io::Write, path::Path};

/// Commands that manage or report on Setup themselves, plus help and version output.
fn is_exempt(args: &[&str]) -> bool {
    matches!(
        args,
        [] | [
            "--help" | "-h" | "--version" | "-V" | "install" | "uninstall" | "doctor",
            ..
        ]
    )
}

/// With no install record, adopts agents whose default skills folder already holds the bundled skills
/// and records them with an unknown version, else prints one hint on `err`. Never fails the command.
pub(crate) fn check(
    args: &[&str],
    var: impl Fn(&str) -> Option<OsString>,
    home: &Path,
    err: &mut impl Write,
) {
    if is_exempt(args) {
        return;
    }
    let Some(path) = install_record::path_in(&var) else {
        return;
    };
    if !matches!(install_record::read(&path), Ok(None)) {
        return;
    }
    let adopted: Vec<String> = ["claude", "codex", "copilot"]
        .into_iter()
        .filter(|agent| {
            crate::install::skills_root(agent, home, &var).is_ok_and(|root| has_bundled(&root))
        })
        .map(String::from)
        .collect();
    let record = InstallRecord {
        version: String::new(),
        agents: adopted,
        claude_config_dirs: Vec::new(),
    };
    if !record.agents.is_empty() && install_record::write(&path, &record).is_ok() {
        return;
    }
    let _ = writeln!(
        err,
        "jidoka: skills are not set up; run `jidoka install` to install them."
    );
}

fn has_bundled(root: &Path) -> bool {
    crate::install::bundled_skill_names()
        .iter()
        .all(|name| root.join(name).join("SKILL.md").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Env {
        home: tempfile::TempDir,
    }

    impl Env {
        fn new() -> Self {
            Self {
                home: tempfile::tempdir().unwrap(),
            }
        }
        fn var(&self, name: &str) -> Option<OsString> {
            match name {
                "XDG_CONFIG_HOME" | "APPDATA" => Some(self.home.path().join("cfg").into()),
                "CODEX_HOME" => Some(self.home.path().join("codex-home").into()),
                _ => None,
            }
        }
        fn record_path(&self) -> std::path::PathBuf {
            install_record::path_in(|n| self.var(n)).unwrap()
        }
        fn run(&self, args: &[&str]) -> String {
            let mut err = Vec::new();
            check(args, |n| self.var(n), self.home.path(), &mut err);
            String::from_utf8(err).unwrap()
        }
        fn bundle(&self, root: &Path) {
            for name in crate::install::bundled_skill_names() {
                fs::create_dir_all(root.join(name)).unwrap();
                fs::write(root.join(name).join("SKILL.md"), "x").unwrap();
            }
        }
    }

    #[test]
    fn without_record_or_skills_one_hint_line_is_printed() {
        let env = Env::new();
        let err = env.run(&["run", "inspect", "1"]);
        assert_eq!(err.lines().count(), 1);
        assert!(err.contains("jidoka install"));
        assert!(!env.record_path().exists());
    }

    #[test]
    fn exempt_commands_print_nothing() {
        let env = Env::new();
        for args in [
            &[][..],
            &["--help"],
            &["-h"],
            &["--version"],
            &["install"],
            &["uninstall", "--agent", "claude"],
            &["doctor"],
        ] {
            assert_eq!(env.run(args), "", "{args:?}");
        }
    }

    #[test]
    fn existing_installs_are_adopted_with_an_unknown_version() {
        let env = Env::new();
        env.bundle(&env.home.path().join(".claude/skills"));
        env.bundle(&env.home.path().join("codex-home/skills"));
        assert_eq!(env.run(&["check"]), "");
        let record = install_record::read(&env.record_path()).unwrap().unwrap();
        assert_eq!(record.agents, ["claude", "codex"]);
        assert_ne!(record.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(record.version, "");
        assert!(record.claude_config_dirs.is_empty());
        assert_eq!(env.run(&["check"]), "");
    }

    #[test]
    fn partial_skills_are_not_adopted() {
        let env = Env::new();
        fs::create_dir_all(env.home.path().join(".claude/skills/jidoka")).unwrap();
        assert!(env.run(&["check"]).contains("jidoka install"));
    }

    #[test]
    fn an_existing_record_is_left_alone_silently() {
        let env = Env::new();
        let record = InstallRecord::current(vec!["copilot".into()], vec![]);
        install_record::write(&env.record_path(), &record).unwrap();
        assert_eq!(env.run(&["check"]), "");
        assert_eq!(
            install_record::read(&env.record_path()).unwrap(),
            Some(record)
        );
    }
}
