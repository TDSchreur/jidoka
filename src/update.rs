//! `factory check` finds out whether a newer release exists; `factory update` installs it.

use crate::install;
use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const LATEST_URL: &str = "https://github.com/TDSchreur/tds-factory2/releases/latest";
const DOWNLOAD_URL: &str = "https://github.com/TDSchreur/tds-factory2/releases/latest/download";
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
        Status::UpToDate => "factory is up to date.".to_string(),
        Status::Available { current, latest } => format!(
            "Update available: {current} -> {latest}. Run 'factory update' to install it."
        ),
        Status::Unknown { latest } => format!(
            "This is a development build, so it cannot be compared. The latest release is {latest}; 'factory update' installs it."
        ),
    }
}

/// The version of the latest release, read from where GitHub redirects `releases/latest`.
pub(crate) fn latest_version() -> Result<String, String> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let output = Command::new("curl")
        .args(["-fsSL", "-o", null, "-w", "%{url_effective}", LATEST_URL])
        .output()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => {
                "curl is required to look up releases but was not found.".to_string()
            }
            _ => format!("Could not run curl: {error}"),
        })?;
    if !output.status.success() {
        return Err(format!(
            "Could not look up the latest release: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let url = String::from_utf8_lossy(&output.stdout);
    version_from_url(&url).ok_or_else(|| format!("Could not read a release version from {url}."))
}

/// Runs `factory check` for a build with this version.
pub(crate) fn check(current: &str, release: bool) -> Result<(), String> {
    let latest = latest_version()?;
    println!("{}", render(&status(current, release, &latest)?));
    Ok(())
}

/// The release asset for this platform.
fn asset_name() -> Result<&'static str, String> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok("factory-linux-x86_64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Ok("factory-windows-x86_64.exe")
    } else {
        Err("'factory update' supports only Linux and Windows on x86-64. Build from source instead.".to_string())
    }
}

/// Downloads `url` to `target` with curl.
fn download(url: &str, target: &Path) -> Result<(), String> {
    let output = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(target)
        .arg(url)
        .output()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => {
                "curl is required to download releases but was not found.".to_string()
            }
            _ => format!("Could not run curl: {error}"),
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not download {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
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
fn replace_executable(current: &Path, new: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(new, fs::Permissions::from_mode(0o755))?;
    }
    // Windows cannot overwrite a running executable but can rename it.
    #[cfg(windows)]
    {
        let old = current.with_extension("old");
        let _ = fs::remove_file(&old);
        fs::rename(current, &old)?;
        if let Err(error) = fs::rename(new, current) {
            let _ = fs::rename(&old, current);
            return Err(error);
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    fs::rename(new, current)
}

/// The agents that already have bundled skills installed.
pub(crate) fn installed_agents(
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> Vec<&'static str> {
    AGENTS
        .into_iter()
        .filter(|agent| {
            install::skills_root(agent, home, &var).is_ok_and(|root| {
                install::bundled_skill_files().any(|file| root.join(file).is_file())
            })
        })
        .collect()
}

/// Runs `factory update`: installs the latest release over this executable, then its skills.
pub(crate) fn update(current: &str, release: bool, agents: Option<&str>) -> Result<(), String> {
    let latest = latest_version()?;
    if status(current, release, &latest)? == Status::UpToDate {
        println!("factory {current} is already the latest release.");
        return Ok(());
    }
    let agents = match agents {
        Some(agents) => {
            let home = install::home_dir().map_err(|error| error.to_string())?;
            for agent in agents.split(',').map(str::trim) {
                install::skills_root(agent, &home, |name| env::var_os(name))
                    .map_err(|error| error.to_string())?;
            }
            agents.to_string()
        }
        None => {
            let home = install::home_dir().map_err(|error| error.to_string())?;
            installed_agents(&home, |name| env::var_os(name)).join(",")
        }
    };
    let asset = asset_name()?;
    let exe =
        env::current_exe().map_err(|error| format!("Cannot find this executable: {error}"))?;
    let dir = exe
        .parent()
        .ok_or("Cannot find this executable's folder.")?;
    // Stage next to the executable so the final rename stays on one filesystem.
    let staged: PathBuf = dir.join(format!(".factory-update-{}", std::process::id()));
    let checksum = staged.with_extension("sha256");
    let result = (|| {
        println!("Downloading factory {latest}...");
        download(&format!("{DOWNLOAD_URL}/{asset}"), &staged)?;
        download(&format!("{DOWNLOAD_URL}/{asset}.sha256"), &checksum)?;
        let data = fs::read(&staged).map_err(|error| error.to_string())?;
        let checksum = fs::read_to_string(&checksum).map_err(|error| error.to_string())?;
        verify_checksum(&data, &checksum)?;
        replace_executable(&exe, &staged)
            .map_err(|error| format!("Cannot replace {}: {error}", exe.display()))
    })();
    let _ = fs::remove_file(&staged);
    let _ = fs::remove_file(&checksum);
    result?;
    println!("Updated {} to factory {latest}.", exe.display());
    if agents.is_empty() {
        println!("No installed skills found. Run 'factory install --agent <codex|claude|copilot>' to install them.");
    } else {
        // The new binary carries the new skills, so it installs them.
        let installed = Command::new(&exe)
            .args(["install", "--agent", &agents])
            .status()
            .map_err(|error| format!("Could not run the new factory: {error}"))?;
        if !installed.success() {
            return Err(format!(
                "factory was updated, but installing the skills failed. Run 'factory install --agent {agents}'."
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
            version_from_url("https://github.com/TDSchreur/tds-factory2/releases/tag/v0.5.1\n"),
            Some("0.5.1".to_string())
        );
        for bad in [
            "https://github.com/TDSchreur/tds-factory2/releases",
            "https://github.com/TDSchreur/tds-factory2/releases/tag/nightly",
            "https://github.com/x/releases/tag/v1.2",
            "",
        ] {
            assert_eq!(version_from_url(bad), None, "{bad}");
        }
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
        assert!(render(&Status::Unknown {
            latest: "0.6.0".into()
        })
        .contains("0.6.0"));
    }

    #[test]
    fn a_matching_checksum_is_accepted_and_anything_else_is_not() {
        // SHA-256 of "abc".
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  factory\n";
        assert!(verify_checksum(b"abc", good).is_ok());
        assert!(verify_checksum(b"abd", good).is_err());
        assert!(verify_checksum(b"abc", "").is_err());
    }

    #[test]
    fn replacing_the_executable_swaps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (current, new) = (dir.path().join("factory"), dir.path().join("staged"));
        fs::write(&current, "old").unwrap();
        fs::write(&new, "new").unwrap();
        replace_executable(&current, &new).unwrap();
        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
    }

    #[test]
    fn only_agents_with_installed_skills_are_updated() {
        let home = tempfile::tempdir().unwrap();
        let none = |_: &str| None;
        assert!(installed_agents(home.path(), none).is_empty());
        let file = install::bundled_skill_files().next().unwrap();
        let path = home.path().join(".claude/skills").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
        assert_eq!(installed_agents(home.path(), none), ["claude"]);
    }
}
