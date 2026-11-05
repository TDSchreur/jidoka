//! `factory check`: finds out whether a newer release exists.

use std::{io, process::Command};

const LATEST_URL: &str = "https://github.com/TDSchreur/tds-factory2/releases/latest";

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
}
