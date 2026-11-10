//! Remove selected bundled skills and the installer-managed command.
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
};

pub(crate) fn run(options: &[&str]) -> Result<(), String> {
    let home = crate::install::home_dir().map_err(|error| error.to_string())?;
    let current_exe = env::current_exe().ok();
    uninstall(
        options,
        |name| env::var_os(name),
        &home,
        current_exe.as_deref(),
        confirm_run_data,
    )
}

fn confirm_run_data() -> Result<bool, String> {
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("Remove Jidoka run data and the model routing policy? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|error| format!("cannot display confirmation: {error}"))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("cannot read confirmation: {error}"))?;
    Ok(answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes"))
}

fn uninstall(
    options: &[&str],
    var: impl Fn(&str) -> Option<OsString>,
    home: &Path,
    current_exe: Option<&Path>,
    confirm: impl FnOnce() -> Result<bool, String>,
) -> Result<(), String> {
    let keep_data = options.contains(&"--keep-data");
    let filtered: Vec<_> = options
        .iter()
        .copied()
        .filter(|option| *option != "--keep-data")
        .collect();
    let (agent_flag, dir_flags) = crate::install::parse_options(&filtered)?;
    let record_path = crate::install_record::path_in(&var);
    let (agents, dirs, record_for_partial) = match agent_flag {
        Some(agent) => {
            let record = match &record_path {
                Some(path) => crate::install_record::read(path).ok().flatten(),
                None => None,
            };
            (agent.to_string(), dir_flags, record)
        }
        None => {
            let record = match &record_path {
                Some(path) => crate::install_record::read(path).map_err(|error| {
                    format!("{error}; fix or delete that file, or pass --agent")
                })?,
                None => None,
            };
            let record = record.ok_or("uninstall needs --agent (no install record found)")?;
            let dirs = if dir_flags.is_empty() {
                record.claude_config_dirs.clone()
            } else {
                dir_flags
            };
            let agents = record.agents.join(",");
            (agents, dirs, Some(record))
        }
    };
    let remaining = record_for_partial
        .as_ref()
        .and_then(|record| remaining_record(record, &agents, &dirs));
    let roots = crate::install::skills_roots(&agents, &dirs, home, &var)
        .map_err(|error| format!("cannot select skills: {error}"))?;
    for root in roots {
        for name in crate::install::bundled_skill_names() {
            remove(&root.join(name))?;
        }
    }
    if let (Some(remaining), Some(record_path)) = (&remaining, &record_path) {
        crate::install_record::write(record_path, remaining)
            .map_err(|error| format!("cannot update {}: {error}", record_path.display()))?;
        println!(
            "Agents that remain set up: {}. Run `jidoka uninstall` again to remove the rest.",
            remaining.agents.join(", ")
        );
        return Ok(());
    }
    if !keep_data && confirm()? {
        remove_run_data(&var)?;
        if let Some(policy) = crate::routing::policy_path_in(&var) {
            remove(&policy)?;
        }
    }
    if let Some(record_path) = record_path {
        remove(&record_path)?;
        remove(&record_path.with_file_name("install.lock"))?;
    }
    let target = script_location(home, &var)?;
    if current_exe.is_some_and(|current| same_file(current, &target)) {
        remove_executable(&target)?;
    } else {
        println!("{FOREIGN_BINARY_NOTE}");
    }
    Ok(())
}

/// The record minus what this uninstall removes, or `None` when nothing remains.
///
/// Removing `claude` without naming config directories removes it entirely; naming directories
/// keeps `claude` set up for the recorded directories that were not named.
fn remaining_record(
    record: &crate::install_record::InstallRecord,
    removed_agents: &str,
    removed_dirs: &[String],
) -> Option<crate::install_record::InstallRecord> {
    let removed: Vec<&str> = removed_agents.split(',').collect();
    let mut agents = Vec::new();
    let mut dirs = Vec::new();
    for agent in &record.agents {
        if !removed.contains(&agent.as_str()) {
            agents.push(agent.clone());
            if agent == "claude" {
                dirs.clone_from(&record.claude_config_dirs);
            }
        } else if agent == "claude" && !removed_dirs.is_empty() {
            dirs = record
                .claude_config_dirs
                .iter()
                .filter(|dir| !removed_dirs.contains(dir))
                .cloned()
                .collect();
            if !dirs.is_empty() {
                agents.push(agent.clone());
            }
        }
    }
    (!agents.is_empty()).then(|| crate::install_record::InstallRecord {
        version: record.version.clone(),
        agents,
        claude_config_dirs: dirs,
    })
}

/// Printed when the running binary is not the one the install script placed.
#[cfg(windows)]
const FOREIGN_BINARY_NOTE: &str = "Jidoka was not installed by the install script; remove the binary with `winget uninstall TDSchreur.Jidoka`.";
#[cfg(not(windows))]
const FOREIGN_BINARY_NOTE: &str = "Jidoka was not installed by the install script; remove the binary with the package manager that installed it.";

/// Where the install scripts put the executable.
#[cfg(not(windows))]
fn script_location(
    home: &Path,
    _var: &impl Fn(&str) -> Option<OsString>,
) -> Result<std::path::PathBuf, String> {
    Ok(home.join(".local/bin/jidoka"))
}

#[cfg(windows)]
fn script_location(
    _home: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
) -> Result<std::path::PathBuf, String> {
    let base = var("LOCALAPPDATA").ok_or("cannot find LOCALAPPDATA")?;
    Ok(std::path::PathBuf::from(base)
        .join("Programs")
        .join("jidoka")
        .join("jidoka.exe"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

fn inspect(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn remove(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    let result = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
    println!("Removed: {}", path.display());
    Ok(())
}

fn remove_run_data(var: &impl Fn(&str) -> Option<OsString>) -> Result<(), String> {
    let path = crate::data_path_in(crate::server::DEFAULT_PORT, var)
        .map_err(|error| format!("cannot find run data: {error}"))?;
    let dir = path.parent().ok_or("cannot find run data directory")?;
    if inspect(dir)?.is_some_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(format!("run data directory {} is a symlink", dir.display()));
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot read {}: {error}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let port_suffix = |prefix: &str, suffix: &str| {
            name.strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(suffix))
                .is_some_and(|s| !s.is_empty() && s.parse::<u16>().is_ok())
        };
        if name == "runs.json"
            || name == "plans"
            || name == "session-token"
            || port_suffix("runs-", ".json")
            || port_suffix("plans-", "")
            || port_suffix("session-token-", "")
        {
            remove(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_executable(target: &Path) -> Result<(), String> {
    remove(target)
}

/// Removes entries equal to `bin` (ignoring case and trailing backslashes) from a
/// `;`-separated PATH value. Returns `None` when nothing matched.
#[cfg(any(windows, test))]
fn filter_path_entries(value: &str, bin: &str) -> Option<String> {
    let normalize = |entry: &str| entry.trim_end_matches('\\').to_lowercase();
    let wanted = normalize(bin);
    let entries: Vec<&str> = value.split(';').collect();
    let kept: Vec<&str> = entries
        .iter()
        .copied()
        .filter(|entry| normalize(entry) != wanted)
        .collect();
    (kept.len() != entries.len()).then(|| kept.join(";"))
}

/// Builds a paste-ready PowerShell command that deletes `path`.
///
/// Every single quote PowerShell accepts as a delimiter (ASCII and typographic) is doubled.
#[cfg(any(windows, test))]
fn delete_command(path: &str) -> String {
    let mut quoted = String::with_capacity(path.len());
    for c in path.chars() {
        quoted.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            quoted.push(c);
        }
    }
    format!("Remove-Item -LiteralPath '{quoted}'")
}

#[cfg(windows)]
fn remove_executable(target: &Path) -> Result<(), String> {
    let bin = target.parent().ok_or("cannot find the install directory")?;
    if registry::remove_from_user_path(registry::ENVIRONMENT_SUBKEY, &bin.to_string_lossy())? {
        registry::broadcast_environment_change();
    }
    // The running executable cannot delete itself on Windows.
    println!(
        "jidoka.exe is still running and was left in place: {}\nDelete it after this command exits by running this in PowerShell:\n  {}",
        target.display(),
        delete_command(&target.to_string_lossy())
    );
    Ok(())
}

#[cfg(windows)]
mod registry {
    use super::filter_path_entries;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::ERROR_FILE_NOT_FOUND,
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
        },
        UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        },
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Subkey of `HKEY_CURRENT_USER` that holds the user environment.
    pub(super) const ENVIRONMENT_SUBKEY: &str = "Environment";
    /// Name of the user PATH value inside the environment subkey.
    const PATH_VALUE: &str = "Path";
    /// Longest time, in milliseconds, one hung window may delay the environment broadcast.
    const BROADCAST_TIMEOUT_MS: u32 = 5000;

    /// Turns a Win32 status code into a readable error.
    fn os_error(status: u32) -> std::io::Error {
        std::io::Error::from_raw_os_error(i32::try_from(status).unwrap_or(i32::MAX))
    }

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by RegOpenKeyExW and is closed exactly once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    /// Reads the raw `Path` value (never expanded) and its registry type.
    fn read_path(key: &Key) -> Result<Option<(u32, String)>, String> {
        let name = wide(PATH_VALUE);
        let mut kind = 0u32;
        let mut size = 0u32;
        // SAFETY: `name` is NUL-terminated and outlives the call; a null data pointer only queries the size.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                null(),
                &mut kind,
                null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(format!("cannot read user PATH: {}", os_error(status)));
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Ok(None);
        }
        let mut data = vec![0u16; (size as usize).div_ceil(2)];
        let mut byte_len = u32::try_from(data.len() * 2).map_err(|e| e.to_string())?;
        // SAFETY: `data` provides `byte_len` writable bytes and `name` is NUL-terminated.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                null(),
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut byte_len,
            )
        };
        if status != 0 {
            return Err(format!("cannot read user PATH: {}", os_error(status)));
        }
        data.truncate(byte_len as usize / 2);
        while data.last() == Some(&0) {
            data.pop();
        }
        let text = String::from_utf16(&data).map_err(|_| "user PATH is not valid UTF-16")?;
        Ok(Some((kind, text)))
    }

    fn write_path(key: &Key, kind: u32, value: &str) -> Result<(), String> {
        let name = wide(PATH_VALUE);
        let data = wide(value);
        let byte_len = u32::try_from(data.len() * 2).map_err(|e| e.to_string())?;
        // SAFETY: `data` is a NUL-terminated buffer of `byte_len` bytes and `name` is NUL-terminated.
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                kind,
                data.as_ptr().cast(),
                byte_len,
            )
        };
        if status != 0 {
            return Err(format!("cannot write user PATH: {}", os_error(status)));
        }
        Ok(())
    }

    /// Removes `bin` from the `Path` value under `HKCU\<subkey>`, keeping its type and
    /// leaving `%VARS%` unexpanded. Returns whether the value changed.
    ///
    /// # Errors
    ///
    /// Returns a message when the key cannot be opened, or the value cannot be read or written.
    pub(super) fn remove_from_user_path(subkey: &str, bin: &str) -> Result<bool, String> {
        let subkey = wide(subkey);
        let mut handle: HKEY = null_mut();
        // SAFETY: `subkey` is NUL-terminated and `handle` is a valid out pointer.
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut handle,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        if status != 0 {
            return Err(format!(
                "cannot open user environment: {}",
                os_error(status)
            ));
        }
        let key = Key(handle);
        let Some((kind, value)) = read_path(&key)? else {
            return Ok(false);
        };
        let Some(updated) = filter_path_entries(&value, bin) else {
            return Ok(false);
        };
        write_path(&key, kind, &updated)?;
        Ok(true)
    }

    pub(super) fn broadcast_environment_change() {
        let environment = wide(ENVIRONMENT_SUBKEY);
        // SAFETY: `environment` is a NUL-terminated wide string that outlives the call.
        unsafe {
            // Best effort: a failed or timed-out broadcast only delays new shells seeing the change.
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                BROADCAST_TIMEOUT_MS,
                null_mut(),
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::System::Registry::{
            KEY_ALL_ACCESS, REG_OPTION_NON_VOLATILE, RegCreateKeyExW, RegDeleteKeyW,
        };

        struct Scratch(Vec<u16>);

        impl Drop for Scratch {
            fn drop(&mut self) {
                // SAFETY: the subkey name is NUL-terminated.
                unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, self.0.as_ptr()) };
            }
        }

        #[test]
        fn keeps_registry_type_and_does_not_expand_variables() {
            let path = format!(
                r"Software\jidoka-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let subkey = wide(&path);
            let scratch = Scratch(subkey.clone());
            let mut handle: HKEY = null_mut();
            // SAFETY: `subkey` is NUL-terminated and `handle` is a valid out pointer.
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    0,
                    null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_ALL_ACCESS,
                    null(),
                    &mut handle,
                    null_mut(),
                )
            };
            assert_eq!(status, 0, "creating the scratch registry key failed");
            let key = Key(handle);
            write_path(
                &key,
                REG_EXPAND_SZ,
                r"%USERPROFILE%\bin;C:\Apps\Jidoka\;C:\Last",
            )
            .unwrap();
            assert!(remove_from_user_path(&path, r"C:\Apps\Jidoka").unwrap());
            let (kind, value) = read_path(&key).unwrap().unwrap();
            assert_eq!(kind, REG_EXPAND_SZ);
            assert_eq!(value, r"%USERPROFILE%\bin;C:\Last");
            assert!(!remove_from_user_path(&path, r"C:\Apps\Jidoka").unwrap());
            drop(key);
            drop(scratch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{delete_command, filter_path_entries, uninstall};
    use crate::install_record::{self, InstallRecord};
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
    };

    struct Env {
        dir: tempfile::TempDir,
    }

    impl Env {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }
        fn home(&self) -> PathBuf {
            self.dir.path().join("home")
        }
        fn var(&self, name: &str) -> Option<OsString> {
            let base = self.dir.path();
            match name {
                "HOME" | "USERPROFILE" => Some(self.home().into()),
                "XDG_CONFIG_HOME" | "APPDATA" => Some(base.join("config").into()),
                "XDG_DATA_HOME" | "LOCALAPPDATA" => Some(base.join("data").into()),
                _ => None,
            }
        }
        fn record_path(&self) -> PathBuf {
            install_record::path_in(|name| self.var(name)).unwrap()
        }
        fn policy_path(&self) -> PathBuf {
            self.record_path().with_file_name("model-routing.json")
        }
        fn runs_path(&self) -> PathBuf {
            crate::data_path_in(crate::server::DEFAULT_PORT, &|name| self.var(name)).unwrap()
        }
        fn write(path: &Path) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        fn record(&self, agents: &[&str], dirs: &[&PathBuf]) {
            let agents = agents.iter().map(ToString::to_string).collect();
            let dirs = dirs
                .iter()
                .map(|d| d.to_str().unwrap().to_string())
                .collect();
            install_record::write(&self.record_path(), &InstallRecord::current(agents, dirs))
                .unwrap();
        }
        fn skill(&self, config_dir: &Path) -> PathBuf {
            let name = crate::install::bundled_skill_names()[0];
            let skill = config_dir.join("skills").join(name).join("SKILL.md");
            Self::write(&skill);
            skill
        }
        fn run(
            &self,
            options: &[&str],
            current_exe: Option<&Path>,
            answer: bool,
        ) -> Result<(), String> {
            uninstall(
                options,
                |name| self.var(name),
                &self.home(),
                current_exe,
                || Ok(answer),
            )
        }
    }

    #[test]
    fn filters_only_matching_path_entries() {
        assert_eq!(
            filter_path_entries(
                r"C:\Other;C:\APPS\JIDOKA\;C:\Apps\JidokaMore;C:\Apps\Jidoka;C:\Last",
                r"C:\Apps\Jidoka"
            )
            .as_deref(),
            Some(r"C:\Other;C:\Apps\JidokaMore;C:\Last")
        );
    }

    #[test]
    fn keeps_path_untouched_without_match() {
        assert_eq!(
            filter_path_entries(r"C:\Other;;C:\Apps\JidokaMore", r"C:\Apps\Jidoka"),
            None
        );
    }

    #[test]
    fn keeps_empty_entries_verbatim() {
        assert_eq!(
            filter_path_entries(r"C:\A;;C:\Apps\Jidoka;", r"C:\Apps\Jidoka").as_deref(),
            Some(r"C:\A;;")
        );
    }

    #[test]
    fn delete_command_doubles_single_quotes() {
        assert_eq!(
            delete_command(r"C:\Users\O'Neil\jidoka.exe"),
            r"Remove-Item -LiteralPath 'C:\Users\O''Neil\jidoka.exe'"
        );
    }

    #[test]
    fn delete_command_doubles_typographic_single_quotes() {
        assert_eq!(
            delete_command("C:\\Users\\O\u{2019}Neil\\jidoka.exe"),
            "Remove-Item -LiteralPath 'C:\\Users\\O\u{2019}\u{2019}Neil\\jidoka.exe'"
        );
    }

    #[test]
    fn without_flags_the_record_decides_and_is_removed_with_its_lock() {
        let env = Env::new();
        let dir = env.dir.path().join("cc");
        let skill = env.skill(&dir);
        env.record(&["claude"], &[&dir]);
        Env::write(&env.record_path().with_file_name("install.lock"));
        env.run(&[], None, false).unwrap();
        assert!(!skill.exists());
        assert!(!env.record_path().exists());
        assert!(!env.record_path().with_file_name("install.lock").exists());
    }

    #[test]
    fn flags_override_the_record_and_leave_it_when_agents_remain() {
        let env = Env::new();
        let dir = env.dir.path().join("cc");
        let skill = env.skill(&dir);
        env.record(&["claude"], &[&dir]);
        env.run(&["--agent", "codex"], None, false).unwrap();
        assert!(skill.exists());
        assert!(env.record_path().exists());
    }

    #[test]
    fn without_record_and_agent_it_is_an_error_naming_the_flag() {
        let env = Env::new();
        let error = env.run(&[], None, false).unwrap_err();
        assert!(error.contains("--agent"), "{error}");
    }

    #[test]
    fn the_policy_follows_the_run_data_answer() {
        let env = Env::new();
        for (answer, kept) in [(true, false), (false, true)] {
            Env::write(&env.policy_path());
            Env::write(&env.runs_path());
            env.run(&["--agent", "codex"], None, answer).unwrap();
            assert_eq!(env.policy_path().exists(), kept);
            assert_eq!(env.runs_path().exists(), kept);
        }
    }

    #[test]
    fn keep_data_keeps_the_policy_without_asking() {
        let env = Env::new();
        Env::write(&env.policy_path());
        uninstall(
            &["--agent", "codex", "--keep-data"],
            |name| env.var(name),
            &env.home(),
            None,
            || panic!("must not ask"),
        )
        .unwrap();
        assert!(env.policy_path().exists());
    }

    #[test]
    fn the_package_manager_note_fits_the_platform() {
        assert_eq!(super::FOREIGN_BINARY_NOTE.contains("winget"), cfg!(windows));
    }

    #[test]
    fn a_broken_record_does_not_stop_uninstall_with_an_agent() {
        let env = Env::new();
        let dir = env.dir.path().join("cc");
        let skill = env.skill(&dir);
        Env::write(&env.record_path());
        fs::write(env.record_path(), "{").unwrap();
        let error = env.run(&[], None, false).unwrap_err();
        assert!(error.contains("install.json"), "{error}");
        assert!(error.contains("--agent"), "{error}");
        env.run(
            &["--agent", "claude", "--config-dir", dir.to_str().unwrap()],
            None,
            false,
        )
        .unwrap();
        assert!(!skill.exists());
        assert!(!env.record_path().exists());
    }

    #[test]
    #[cfg(windows)]
    fn the_script_location_is_under_localappdata() {
        let env = Env::new();
        let expected = env.dir.path().join("data/Programs/jidoka/jidoka.exe");
        assert_eq!(
            super::script_location(&env.home(), &|name| env.var(name)).unwrap(),
            expected
        );
        assert!(super::script_location(&env.home(), &|_| None).is_err());
    }

    #[test]
    #[cfg(not(windows))]
    fn only_the_executable_at_the_script_location_is_deleted() {
        let env = Env::new();
        let installed = env.home().join(".local/bin/jidoka");
        Env::write(&installed);
        let other = env.dir.path().join("pkg/jidoka");
        Env::write(&other);
        env.run(&["--agent", "codex"], Some(&other), false).unwrap();
        assert!(installed.exists());
        assert!(other.exists());
        env.run(&["--agent", "codex"], Some(&installed), false)
            .unwrap();
        assert!(!installed.exists());
    }
}
