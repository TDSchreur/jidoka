//! Remove selected bundled skills and the installer-managed command.
use std::{
    env, fs,
    io::{self, IsTerminal, Write},
    path::Path,
};

pub(crate) fn run(options: &[&str]) -> Result<(), String> {
    let keep_data = options.contains(&"--keep-data");
    let filtered: Vec<_> = options
        .iter()
        .copied()
        .filter(|option| *option != "--keep-data")
        .collect();
    let (agents, dirs) = crate::install::parse_options(&filtered)?;
    let agents = agents.ok_or("uninstall needs --agent")?;
    let home = crate::install::home_dir().map_err(|error| error.to_string())?;
    let roots = crate::install::skills_roots(agents, &dirs, &home, |name| env::var_os(name))
        .map_err(|error| format!("cannot select skills: {error}"))?;
    for root in roots {
        for name in crate::install::bundled_skill_names() {
            remove(&root.join(name))?;
        }
    }
    if !keep_data && io::stdin().is_terminal() {
        print!("Remove Jidoka run data? [y/N] ");
        io::stdout()
            .flush()
            .map_err(|error| format!("cannot display confirmation: {error}"))?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|error| format!("cannot read confirmation: {error}"))?;
        if answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes") {
            remove_run_data()?;
        }
    }
    remove_executable(&home)?;
    Ok(())
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

fn remove_run_data() -> Result<(), String> {
    let path = crate::data_path(crate::server::DEFAULT_PORT)
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
fn remove_executable(home: &Path) -> Result<(), String> {
    let target = home.join(".local/bin/jidoka");
    // Only the fixed installer destination is eligible, never a command found on PATH.
    if inspect(&target)?.is_some_and(|metadata| metadata.file_type().is_file()) {
        remove(&target)?;
    }
    Ok(())
}

#[cfg(windows)]
const FILTER_PATH: &str = r"$all = @($p -split ';'); $parts = @($all | Where-Object { $_.TrimEnd('\') -ine $env:JIDOKA_UNINSTALL_BIN.TrimEnd('\') })";

#[cfg(windows)]
const REMOVE_AFTER_EXIT: &str = r"$ErrorActionPreference = 'Stop'; Wait-Process -Id $env:JIDOKA_UNINSTALL_PID -ErrorAction SilentlyContinue; Remove-Item -LiteralPath $env:JIDOKA_UNINSTALL_EXE -Force";

#[cfg(windows)]
fn remove_executable(_home: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let base = env::var_os("LOCALAPPDATA").ok_or("cannot find LOCALAPPDATA")?;
    let bin = std::path::PathBuf::from(base)
        .join("Programs")
        .join("jidoka");
    let target = bin.join("jidoka.exe");
    let current =
        env::current_exe().map_err(|error| format!("cannot locate running command: {error}"))?;
    let running_installed = match fs::canonicalize(&target) {
        Ok(path) => path == current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("cannot inspect {}: {error}", target.display())),
    };
    let powershell =
        std::path::PathBuf::from(env::var_os("SystemRoot").ok_or("cannot find SystemRoot")?)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
    let path_script = format!(
        "$ErrorActionPreference = 'Stop'; $p = [Environment]::GetEnvironmentVariable('Path', 'User'); if ($null -ne $p) {{ {FILTER_PATH}; if ($parts.Count -ne $all.Count) {{ [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User') }} }}"
    );
    let status = Command::new(&powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", &path_script])
        .env("JIDOKA_UNINSTALL_BIN", &bin)
        .status()
        .map_err(|error| format!("cannot start PowerShell to update user PATH: {error}"))?;
    if !status.success() {
        return Err(format!("cannot remove {} from user PATH", bin.display()));
    }
    if !running_installed {
        if inspect(&target)?.is_some_and(|metadata| metadata.file_type().is_file()) {
            remove(&target)?;
        }
        return Ok(());
    }
    Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            REMOVE_AFTER_EXIT,
        ])
        .env("JIDOKA_UNINSTALL_PID", std::process::id().to_string())
        .env("JIDOKA_UNINSTALL_EXE", &target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot schedule removal of {}: {error}", target.display()))?;
    println!("Scheduled removal: {}", target.display());
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::{FILTER_PATH, REMOVE_AFTER_EXIT};
    use std::{
        fs,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    fn powershell() -> Command {
        let root = std::env::var_os("SystemRoot").expect("SystemRoot");
        Command::new(
            std::path::PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        )
    }

    #[test]
    fn filters_only_matching_path_entries() {
        let script = format!(
            "$p = $env:JIDOKA_TEST_PATH; {FILTER_PATH}; [Console]::Out.Write(($parts -join ';'))"
        );
        let output = powershell()
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .env("JIDOKA_UNINSTALL_BIN", r"C:\Apps\Jidoka")
            .env(
                "JIDOKA_TEST_PATH",
                r"C:\Other;C:\APPS\JIDOKA\;C:\Apps\JidokaMore;C:\Apps\Jidoka;C:\Last",
            )
            .output()
            .expect("run PATH filter");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            r"C:\Other;C:\Apps\JidokaMore;C:\Last"
        );
    }

    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn deletes_executable_only_after_process_exits() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("jidoka [test].exe");
        fs::write(&target, "fixture").unwrap();
        let mut process = Process(
            powershell()
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$null = [Console]::ReadLine()",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let ready = dir.path().join("ready");
        let script = format!(
            "function Wait-Process {{ param($Id, $ErrorAction); [IO.File]::WriteAllText($env:JIDOKA_TEST_READY, 'waiting'); Microsoft.PowerShell.Management\\Wait-Process -Id $Id -ErrorAction $ErrorAction }}; {REMOVE_AFTER_EXIT}"
        );
        let mut cleanup = Process(
            powershell()
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .env("JIDOKA_TEST_READY", &ready)
                .env("JIDOKA_UNINSTALL_PID", process.0.id().to_string())
                .env("JIDOKA_UNINSTALL_EXE", &target)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.exists() {
            assert!(
                cleanup.0.try_wait().unwrap().is_none(),
                "cleanup exited before becoming ready"
            );
            assert!(Instant::now() < deadline, "cleanup did not become ready");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            target.exists(),
            "target was removed while the process was running: {}",
            target.display()
        );
        assert!(
            cleanup.0.try_wait().unwrap().is_none(),
            "cleanup exited before the process"
        );
        drop(process.0.stdin.take());
        assert!(
            process.0.wait().unwrap().success(),
            "fixture process failed to exit successfully"
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = cleanup.0.try_wait().unwrap() {
                assert!(status.success(), "cleanup process failed: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "cleanup process did not finish");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !target.exists(),
            "cleanup did not remove {}",
            target.display()
        );
    }
}
