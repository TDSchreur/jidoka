//! `jidoka check` finds out whether a newer release exists; `jidoka update` installs it.

use crate::install;
use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const LATEST_URL: &str = "https://github.com/TDSchreur/jidoka/releases/latest";
const DOWNLOAD_URL: &str = "https://github.com/TDSchreur/jidoka/releases/latest/download";
const AGENTS: [&str; 3] = ["codex", "claude", "copilot"];

/// How the running build compares with the latest release.
#[derive(Debug, PartialEq)]
pub(crate) enum Status {
    UpToDate,
    Available {
        current: String,
        latest: String,
    },
    /// A development build has no release version to compare.
    Unknown {
        latest: String,
    },
}

/// An agent with the platform's TLS: Schannel and the Windows certificate store on Windows,
/// rustls elsewhere.
fn agent(config: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>) -> ureq::Agent {
    #[cfg(windows)]
    let config = config.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    );
    config.build().into()
}

/// The version in a release URL such as `.../releases/tag/v0.5.1`.
pub(crate) fn version_from_url(url: &str) -> Option<String> {
    let tag = url.trim().rsplit_once("/releases/tag/")?.1;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    parse_version(version)?;
    Some(version.to_string())
}

fn parse_version(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.').map(|part| part.parse().ok());
    let version = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(version)
}

pub(crate) fn status(current: &str, release: bool, latest: &str) -> Result<Status, String> {
    let newest = parse_version(latest).ok_or(format!("Unexpected release version {latest:?}."))?;
    if !release {
        return Ok(Status::Unknown {
            latest: latest.to_string(),
        });
    }
    let running = parse_version(current).ok_or(format!("Unexpected version {current:?}."))?;
    Ok(if newest > running {
        Status::Available {
            current: current.to_string(),
            latest: latest.to_string(),
        }
    } else {
        Status::UpToDate
    })
}

pub(crate) fn render(status: &Status) -> String {
    match status {
        Status::UpToDate => "jidoka is up to date.".to_string(),
        Status::Available { current, latest } => {
            format!("Update available: {current} -> {latest}. Run 'jidoka update' to install it.")
        }
        Status::Unknown { latest } => format!(
            "This is a development build, so it cannot be compared. The latest release is {latest}; 'jidoka update' installs it."
        ),
    }
}

/// The version of the latest release, read from where GitHub redirects `releases/latest`.
pub(crate) fn latest_version() -> Result<String, String> {
    let agent = agent(
        ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false),
    );
    let response = agent
        .get(LATEST_URL)
        .call()
        .map_err(|error| request_error("look up the latest release", &error))?;
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok());
    version_from_redirect(response.status().as_u16(), location)
}

/// The release version in a `releases/latest` response, given its status and `Location` header.
fn version_from_redirect(status: u16, location: Option<&str>) -> Result<String, String> {
    if !(300..400).contains(&status) {
        return Err(format!(
            "Could not look up the latest release: GitHub answered with status {status}."
        ));
    }
    let location = location.ok_or("Could not look up the latest release: no redirect location.")?;
    version_from_url(location)
        .ok_or_else(|| format!("Could not read a release version from {location}."))
}

/// A readable message for a failed request while trying to `action`.
fn request_error(action: &str, error: &ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(status) => {
            format!("Could not {action}: the server answered with status {status}.")
        }
        other => format!("Could not {action}: network error: {other}."),
    }
}

/// Runs `jidoka check` for a build with this version.
pub(crate) fn check(current: &str, release: bool) -> Result<(), String> {
    let latest = latest_version()?;
    println!("{}", render(&status(current, release, &latest)?));
    Ok(())
}

/// The release asset for this platform.
fn asset_name() -> Result<&'static str, String> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok("jidoka-linux-x86_64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Ok("jidoka-windows-x86_64.exe")
    } else {
        Err(
            "'jidoka update' supports only Linux and Windows on x86-64. Build from source instead."
                .to_string(),
        )
    }
}

/// Downloads `url` to `target`, following redirects.
fn download(url: &str, target: &Path) -> Result<(), String> {
    let mut response = agent(ureq::Agent::config_builder())
        .get(url)
        .call()
        .map_err(|error| request_error(&format!("download {url}"), &error))?;
    let mut file = fs::File::create(target)
        .map_err(|error| format!("Cannot write {}: {error}", target.display()))?;
    io::copy(&mut response.body_mut().as_reader(), &mut file)
        .map_err(|error| format!("Could not download {url}: {error}"))?;
    Ok(())
}

/// Checks `data` against the first word of a `.sha256` file.
fn verify_checksum(data: &[u8], checksum_file: &str) -> Result<(), String> {
    let expected = checksum_file.split_whitespace().next().unwrap_or_default();
    let actual: String = Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !expected.is_empty() && actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err("Download checksum mismatch; nothing was changed.".to_string())
    }
}

/// Puts `new` in place of the running executable `current`.
#[cfg(not(windows))]
fn replace_executable(current: &Path, new: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(new, fs::Permissions::from_mode(0o755))?;
    fs::rename(new, current)
}

/// Puts `new` in place of the running executable `current`. Windows cannot overwrite a running
/// executable but can rename it.
#[cfg(windows)]
fn replace_executable(current: &Path, new: &Path) -> io::Result<()> {
    let old = current.with_extension("old");
    // A stale copy from an earlier update may not exist; the rename below reports real failures.
    let _ = fs::remove_file(&old);
    fs::rename(current, &old)?;
    fs::rename(new, current).inspect_err(|_| {
        // Best-effort rollback; the rename error is the one returned.
        let _ = fs::rename(&old, current);
    })
}

/// The agents that already have bundled skills installed.
pub(crate) fn installed_agents(
    home: &Path,
    config_dirs: &[String],
    var: impl Fn(&str) -> Option<OsString>,
) -> Vec<&'static str> {
    let has_skills =
        |root: PathBuf| install::bundled_skill_files().any(|file| root.join(file).is_file());
    AGENTS
        .into_iter()
        .filter(|agent| {
            if *agent == "claude" {
                return install::claude_config_dirs(config_dirs, home, &var)
                    .is_ok_and(|dirs| dirs.into_iter().any(|dir| has_skills(dir.join("skills"))));
            }
            install::skills_root(agent, home, &var).is_ok_and(has_skills)
        })
        .collect()
}

/// The arguments of the `jidoka install` that update runs: the agents, and one `--config-dir` per folder.
pub(crate) fn install_args(agents: &str, claude_dirs: &[PathBuf]) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["install".into(), "--agent".into(), agents.into()];
    for dir in claude_dirs {
        args.push("--config-dir".into());
        args.push(dir.into());
    }
    args
}

/// Runs `jidoka update`: installs the latest release over this executable, then its skills.
pub(crate) fn update(
    current: &str,
    release: bool,
    agents: Option<&str>,
    config_dirs: &[String],
) -> Result<(), String> {
    // Checked first, so a bad `--config-dir` fails before anything is downloaded or written.
    let home = install::home_dir().map_err(|error| error.to_string())?;
    let claude_dirs = install::claude_config_dirs(config_dirs, &home, |name| env::var_os(name))
        .map_err(|error| error.to_string())?;
    let latest = latest_version()?;
    if status(current, release, &latest)? == Status::UpToDate {
        install::ensure_default_policy(|name| env::var_os(name))
            .map_err(|error| error.to_string())?;
        println!("jidoka {current} is already the latest release.");
        return Ok(());
    }
    let agents = match agents {
        Some(agents) => {
            for agent in agents.split(',').map(str::trim) {
                install::skills_root(agent, &home, |name| env::var_os(name))
                    .map_err(|error| error.to_string())?;
            }
            agents.to_string()
        }
        None => installed_agents(&home, config_dirs, |name| env::var_os(name)).join(","),
    };
    if !config_dirs.is_empty() && !agents.split(',').any(|agent| agent.trim() == "claude") {
        return Err("--config-dir applies to the claude agent, which is not being updated; use --agent claude or run 'jidoka install --agent claude --config-dir <path>'.".into());
    }
    let asset = asset_name()?;
    let exe =
        env::current_exe().map_err(|error| format!("Cannot find this executable: {error}"))?;
    let dir = exe
        .parent()
        .ok_or("Cannot find this executable's folder.")?;
    // Stage next to the executable so the final rename stays on one filesystem.
    let staged: PathBuf = dir.join(format!(".jidoka-update-{}", std::process::id()));
    let checksum = staged.with_extension("sha256");
    let result = (|| {
        println!("Downloading jidoka {latest}...");
        download(&format!("{DOWNLOAD_URL}/{asset}"), &staged)?;
        download(&format!("{DOWNLOAD_URL}/{asset}.sha256"), &checksum)?;
        let data = fs::read(&staged).map_err(|error| error.to_string())?;
        let checksum = fs::read_to_string(&checksum).map_err(|error| error.to_string())?;
        verify_checksum(&data, &checksum)?;
        replace_executable(&exe, &staged)
            .map_err(|error| format!("Cannot replace {}: {error}", exe.display()))
    })();
    // Best-effort cleanup of temporary files; the update result is reported below.
    let _ = fs::remove_file(&staged);
    let _ = fs::remove_file(&checksum);
    result?;
    println!("Updated {} to Jidoka {latest}.", exe.display());
    if agents.is_empty() {
        install::ensure_default_policy(|name| env::var_os(name))
            .map_err(|error| error.to_string())?;
        println!(
            "No installed skills found. Run 'jidoka install --agent <codex|claude|copilot>' to install them."
        );
    } else {
        // The new binary carries the new skills, so it installs them.
        let forwarded = if config_dirs.is_empty() {
            &[][..]
        } else {
            &claude_dirs[..]
        };
        let installed = Command::new(&exe)
            .args(install_args(&agents, forwarded))
            .status()
            .map_err(|error| format!("Could not run the new jidoka: {error}"))?;
        if !installed.success() {
            return Err(format!(
                "jidoka was updated, but installing the skills failed. Run 'jidoka install --agent {agents}' again, with the same --config-dir options."
            ));
        }
    }
    println!("Restart a running dashboard to use the new version.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_comes_from_the_release_url() {
        assert_eq!(
            version_from_url("https://github.com/TDSchreur/jidoka/releases/tag/v0.5.1\n"),
            Some("0.5.1".to_string())
        );
        for bad in [
            "https://github.com/TDSchreur/jidoka/releases",
            "https://github.com/TDSchreur/jidoka/releases/tag/nightly",
            "https://github.com/x/releases/tag/v1.2",
            "",
        ] {
            assert_eq!(version_from_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_redirect_to_a_release_gives_its_version() {
        let location = "https://github.com/TDSchreur/jidoka/releases/tag/v0.15.2";
        assert_eq!(
            version_from_redirect(302, Some(location)),
            Ok("0.15.2".to_string())
        );
    }

    #[test]
    fn a_missing_or_unrelated_redirect_is_an_error() {
        assert!(version_from_redirect(302, None).is_err());
        let error = version_from_redirect(302, Some("https://github.com/login")).unwrap_err();
        assert!(error.contains("https://github.com/login"), "{error}");
    }

    #[test]
    fn a_non_redirect_status_is_an_error_naming_the_status() {
        let location = "https://github.com/TDSchreur/jidoka/releases/tag/v0.15.2";
        for status in [200, 404, 503] {
            let error = version_from_redirect(status, Some(location)).unwrap_err();
            assert!(error.contains(&status.to_string()), "{error}");
        }
    }

    #[test]
    fn request_failures_say_what_went_wrong() {
        let status = request_error("download x", &ureq::Error::StatusCode(404));
        assert!(
            status.contains("404") && status.contains("download x"),
            "{status}"
        );
        let network = request_error("download x", &ureq::Error::HostNotFound);
        assert!(network.contains("network error"), "{network}");
    }

    #[test]
    fn versions_compare_by_number_not_text() {
        let newer =
            |current, latest| matches!(status(current, true, latest), Ok(Status::Available { .. }));
        assert!(newer("0.9.0", "0.10.0"));
        assert!(newer("0.5.0", "1.0.0"));
        assert!(!newer("0.10.0", "0.9.0"));
        assert_eq!(status("0.5.0", true, "0.5.0"), Ok(Status::UpToDate));
        // A build newer than the latest release is not offered a downgrade.
        assert_eq!(status("0.6.0", true, "0.5.0"), Ok(Status::UpToDate));
    }

    #[test]
    fn a_development_build_is_not_compared() {
        assert_eq!(
            status("0.5.0", false, "0.5.0"),
            Ok(Status::Unknown {
                latest: "0.5.0".to_string()
            })
        );
        assert!(status("0.5.0", true, "oops").is_err());
    }

    #[test]
    fn the_messages_name_the_versions() {
        let available = Status::Available {
            current: "0.5.0".into(),
            latest: "0.6.0".into(),
        };
        assert!(render(&available).contains("0.5.0 -> 0.6.0"));
        assert!(render(&Status::UpToDate).contains("up to date"));
        assert!(
            render(&Status::Unknown {
                latest: "0.6.0".into()
            })
            .contains("0.6.0")
        );
    }

    #[test]
    fn a_matching_checksum_is_accepted_and_anything_else_is_not() {
        // SHA-256 of "abc".
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  jidoka\n";
        assert!(verify_checksum(b"abc", good).is_ok());
        assert!(verify_checksum(b"abd", good).is_err());
        assert!(verify_checksum(b"abc", "").is_err());
    }

    #[test]
    fn replacing_the_executable_swaps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (current, new) = (dir.path().join("jidoka"), dir.path().join("staged"));
        fs::write(&current, "old").unwrap();
        fs::write(&new, "new").unwrap();
        replace_executable(&current, &new).unwrap();
        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
    }

    #[test]
    fn only_agents_with_installed_skills_are_updated() {
        let home = tempfile::tempdir().unwrap();
        let none = |_: &str| None;
        assert!(installed_agents(home.path(), &[], none).is_empty());
        let file = install::bundled_skill_files().next().unwrap();
        let path = home.path().join(".claude/skills").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
        assert_eq!(installed_agents(home.path(), &[], none), ["claude"]);
    }

    #[test]
    fn config_dirs_decide_whether_claude_is_installed() {
        let home = tempfile::tempdir().unwrap();
        let work = home.path().join(".claude-work");
        let file = install::bundled_skill_files().next().unwrap();
        let path = work.join("skills").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
        let none = |_: &str| None;
        let dirs = [work.display().to_string()];
        assert!(installed_agents(home.path(), &[], none).is_empty());
        assert_eq!(installed_agents(home.path(), &dirs, none), ["claude"]);
    }

    #[test]
    fn the_child_install_gets_one_config_dir_per_folder() {
        let dirs = [install::test_abs("a"), install::test_abs("b")];
        let args = install_args("codex,claude", &dirs);
        let expected: Vec<OsString> = [
            "install".into(),
            "--agent".into(),
            "codex,claude".into(),
            "--config-dir".into(),
            dirs[0].clone().into_os_string(),
            "--config-dir".into(),
            dirs[1].clone().into_os_string(),
        ]
        .into();
        assert_eq!(args, expected);
        assert_eq!(
            install_args("claude", &[]),
            ["install", "--agent", "claude"]
        );
    }
}
