//! `jidoka check` finds out whether a newer release exists.

const LATEST_URL: &str = "https://github.com/TDSchreur/jidoka/releases/latest";

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

/// Builds an https-only HTTP agent using the platform's TLS stack.
///
/// That is Schannel and the Windows certificate store on Windows, and rustls elsewhere.
fn http_agent(config: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>) -> ureq::Agent {
    let config = config.https_only(true);
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
            format!("Update available: {current} -> {latest}. Re-run the installer to update.")
        }
        Status::Unknown { latest } => format!(
            "This is a development build, so it cannot be compared. The latest release is {latest}; re-run the installer to install it."
        ),
    }
}

/// The version of the latest release, read from where GitHub redirects `releases/latest`.
pub(crate) fn latest_version() -> Result<String, String> {
    let agent = http_agent(
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
}
