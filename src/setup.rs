//! Startup check: adopts skills installed before the install record existed, or hints at `jidoka install`.
use crate::install::Selection;
use crate::install_record::{self, InstallRecord};
use crate::routing::Starter;
use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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
    match install_record::read(&path) {
        Ok(None) => {}
        Ok(Some(record)) => {
            if record.version != env!("CARGO_PKG_VERSION") {
                refresh(&path, record, &var, home, err);
            }
            return;
        }
        Err(error) => {
            let _ = writeln!(
                err,
                "jidoka: cannot read the install record ({error}); run `jidoka install` to fix it."
            );
            return;
        }
    }
    let adopted: Vec<String> = crate::install::AGENTS
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
        refresh(&path, record, &var, home, err);
        return;
    }
    let _ = writeln!(
        err,
        "jidoka: skills are not set up; run `jidoka install` to install them."
    );
}

/// A lock older than this is left by a process that died.
const STALE_LOCK: Duration = Duration::from_secs(300);

/// Holds `install.lock` next to the record until dropped.
struct Lock(PathBuf);

impl Lock {
    /// `None` when another process holds a fresh lock or the lock cannot be created.
    fn take(path: PathBuf) -> Option<Self> {
        Self::take_with(path, file_age)
    }

    /// `take` with the age of a file injected.
    fn take_with(path: PathBuf, age: impl Fn(&Path) -> Option<Duration>) -> Option<Self> {
        Self::take_hooked(path, age, || {})
    }

    /// `take_with` that runs `after_rename` once a stale lock has been moved aside.
    fn take_hooked(
        path: PathBuf,
        age: impl Fn(&Path) -> Option<Duration>,
        after_rename: impl Fn(),
    ) -> Option<Self> {
        let is_stale = |file: &Path| age(file).is_some_and(|age| age > STALE_LOCK);
        for _ in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Some(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if !is_stale(&path) {
                        return None;
                    }
                    // Rename first: only one process can move the stale file away. If the moved
                    // file turns out fresh, another process replaced the stale lock between our
                    // check and the rename, so put it back: claim the free path with a
                    // placeholder (if someone else took it, drop the moved-aside lock), then
                    // rename the original over the placeholder, keeping its mtime. The path is
                    // never free once claimed, so a lock created since is never replaced.
                    let nanos = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |age| age.as_nanos());
                    let aside = path.with_extension(format!("lock.{}.{nanos}", std::process::id()));
                    if fs::rename(&path, &aside).is_err() {
                        return None;
                    }
                    after_rename();
                    if !is_stale(&aside) {
                        let claimed = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path);
                        if claimed.is_ok() && fs::rename(&aside, &path).is_err() {
                            let _ = fs::remove_file(&path);
                        }
                        let _ = fs::remove_file(&aside);
                        return None;
                    }
                    let _ = fs::remove_file(&aside);
                }
                Err(_) => return None,
            }
        }
        None
    }
}

fn file_age(path: &Path) -> Option<Duration> {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()?
        .elapsed()
        .ok()
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Reinstalls the skills for the recorded agents without prompting, then records this version and
/// prints one line. A failure warns once and keeps the old version so the next command retries.
fn refresh(
    path: &Path,
    record: InstallRecord,
    var: &impl Fn(&str) -> Option<OsString>,
    home: &Path,
    err: &mut impl Write,
) {
    let Some(_lock) = Lock::take(path.with_file_name("install.lock")) else {
        return;
    };
    let selection = Selection {
        agents: record.agents.clone(),
        config_dirs: record.claude_config_dirs.clone(),
    };
    let result = crate::install::install_skill_files(&selection, home, var, &mut io::sink())
        .and_then(|()| crate::install::create_default_policy(var))
        .map(|(path, done)| {
            if let Starter::Replaced(off) = done {
                let shown = path.display().to_string();
                let _ = writeln!(err, "{}", Starter::replaced_message(&shown, off, None));
            }
        })
        .and_then(|()| {
            let current = InstallRecord::current(record.agents.clone(), record.claude_config_dirs);
            install_record::write(path, &current)
        });
    match result {
        Ok(()) => {
            let _ = writeln!(
                err,
                "jidoka: updated skills to {} for {}",
                env!("CARGO_PKG_VERSION"),
                record.agents.join(", ")
            );
        }
        Err(error) => {
            let _ = writeln!(
                err,
                "jidoka: could not update skills ({error}); run `jidoka install` to fix it."
            );
        }
    }
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
        let err = env.run(&["check"]);
        assert_eq!(
            err,
            format!(
                "jidoka: updated skills to {} for codex, claude\n",
                env!("CARGO_PKG_VERSION")
            )
        );
        let record = install_record::read(&env.record_path()).unwrap().unwrap();
        assert_eq!(record.agents, ["codex", "claude"]);
        assert_eq!(record.version, env!("CARGO_PKG_VERSION"));
        assert!(record.claude_config_dirs.is_empty());
        assert_eq!(env.run(&["check"]), "");
        assert!(!lock_path(&env).exists());
    }

    #[test]
    fn an_unreadable_record_warns_once_naming_the_file_and_the_fix() {
        let env = Env::new();
        let path = env.record_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{").unwrap();
        let err = env.run(&["check"]);
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(err.contains("install.json"), "{err}");
        assert!(err.contains("jidoka install"), "{err}");
    }

    #[test]
    fn a_refresh_that_replaces_the_policy_says_so_on_stderr() {
        let env = Env::new();
        old_record(&env, "0.0.1", &["claude"]);
        let policy = crate::routing::policy_path_in(|n| env.var(n)).unwrap();
        fs::write(&policy, r#"{"agents":{"claude":{"roles":{}}}}"#).unwrap();
        let err = env.run(&["check"]);
        assert_eq!(err.lines().count(), 2, "{err}");
        assert!(
            err.contains("Replaced old-format model routing policy"),
            "{err}"
        );
        assert!(err.contains("updated skills"), "{err}");
    }

    #[test]
    fn taking_over_a_stale_lock_leaves_no_files_behind() {
        let env = Env::new();
        old_record(&env, "0.0.1", &["claude"]);
        let file = fs::File::create(lock_path(&env)).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        env.run(&["check"]);
        let names: Vec<_> = fs::read_dir(env.record_path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|n| !n.starts_with("install.lock")),
            "{names:?}"
        );
        assert!(names.iter().all(|n| !n.contains(".tmp")), "{names:?}");
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

    fn old_record(env: &Env, version: &str, agents: &[&str]) {
        let record = InstallRecord {
            version: version.into(),
            agents: agents.iter().map(|a| (*a).into()).collect(),
            claude_config_dirs: vec![],
        };
        install_record::write(&env.record_path(), &record).unwrap();
    }

    fn lock_path(env: &Env) -> std::path::PathBuf {
        env.record_path().with_file_name("install.lock")
    }

    #[test]
    fn an_older_record_refreshes_every_recorded_agent_and_prints_one_line() {
        let env = Env::new();
        old_record(&env, "0.0.1", &["claude", "codex"]);
        let err = env.run(&["run", "inspect", "1"]);
        let expected = format!(
            "jidoka: updated skills to {} for claude, codex\n",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(err, expected);
        assert!(has_bundled(&env.home.path().join(".claude/skills")));
        assert!(has_bundled(&env.home.path().join("codex-home/skills")));
        let record = install_record::read(&env.record_path()).unwrap().unwrap();
        assert_eq!(record.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(record.agents, ["claude", "codex"]);
        assert_eq!(env.run(&["run", "inspect", "1"]), "");
        assert!(!lock_path(&env).exists());
    }

    #[test]
    fn an_adopted_record_with_an_empty_version_is_refreshed() {
        let env = Env::new();
        old_record(&env, "", &["copilot"]);
        assert!(env.run(&["check"]).contains("for copilot"));
    }

    #[test]
    fn recorded_claude_config_dirs_are_refreshed() {
        let env = Env::new();
        let dir = env.home.path().join("work-claude");
        let record = InstallRecord {
            version: "0.0.1".into(),
            agents: vec!["claude".into()],
            claude_config_dirs: vec![dir.display().to_string()],
        };
        install_record::write(&env.record_path(), &record).unwrap();
        assert!(env.run(&["check"]).contains("claude"));
        assert!(has_bundled(&dir.join("skills")));
        assert!(!env.home.path().join(".claude/skills").exists());
    }

    #[test]
    fn exempt_commands_do_not_refresh() {
        let env = Env::new();
        old_record(&env, "0.0.1", &["claude"]);
        assert_eq!(env.run(&["doctor"]), "");
        let record = install_record::read(&env.record_path()).unwrap().unwrap();
        assert_eq!(record.version, "0.0.1");
    }

    #[test]
    fn a_failed_refresh_warns_once_keeps_the_old_version_and_releases_the_lock() {
        let env = Env::new();
        old_record(&env, "0.0.1", &["claude"]);
        fs::write(env.home.path().join(".claude"), "a file, not a folder").unwrap();
        let err = env.run(&["check"]);
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(err.contains("jidoka install"));
        let record = install_record::read(&env.record_path()).unwrap().unwrap();
        assert_eq!(record.version, "0.0.1");
        assert!(!lock_path(&env).exists());
        fs::remove_file(env.home.path().join(".claude")).unwrap();
        assert!(env.run(&["check"]).contains("updated skills"));
    }

    #[test]
    fn a_held_lock_skips_the_refresh_silently() {
        let env = Env::new();
        old_record(&env, "0.0.1", &["claude"]);
        fs::write(lock_path(&env), "").unwrap();
        assert_eq!(env.run(&["check"]), "");
        assert!(!env.home.path().join(".claude/skills").exists());
        let record = install_record::read(&env.record_path()).unwrap().unwrap();
        assert_eq!(record.version, "0.0.1");
        assert!(lock_path(&env).exists());
    }

    #[test]
    fn a_stale_lock_is_ignored() {
        let env = Env::new();
        old_record(&env, "0.0.1", &["claude"]);
        let file = fs::File::create(lock_path(&env)).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        assert!(env.run(&["check"]).contains("updated skills"));
        assert!(!lock_path(&env).exists());
    }

    #[test]
    fn a_fresh_lock_renamed_aside_is_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("install.lock");
        fs::write(&path, "other").unwrap();
        let calls = std::cell::Cell::new(0);
        // Stale on the first check, fresh once moved aside: another process replaced it.
        let age = |_: &Path| {
            calls.set(calls.get() + 1);
            Some(if calls.get() == 1 {
                STALE_LOCK * 2
            } else {
                Duration::ZERO
            })
        };
        assert!(Lock::take_with(path.clone(), age).is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), "other");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_put_back_lock_keeps_its_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("install.lock");
        fs::write(&path, "other").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        let calls = std::cell::Cell::new(0);
        let age = |_: &Path| {
            calls.set(calls.get() + 1);
            Some(if calls.get() == 1 {
                STALE_LOCK * 2
            } else {
                Duration::ZERO
            })
        };
        assert!(Lock::take_with(path.clone(), age).is_none());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn a_fresh_lock_is_never_replaced_when_putting_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("install.lock");
        fs::write(&path, "moved").unwrap();
        let calls = std::cell::Cell::new(0);
        let age = |_: &Path| {
            calls.set(calls.get() + 1);
            Some(if calls.get() == 1 {
                STALE_LOCK * 2
            } else {
                Duration::ZERO
            })
        };
        let place_other = || fs::write(&path, "other").unwrap();
        assert!(Lock::take_hooked(path.clone(), age, place_other).is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), "other");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
