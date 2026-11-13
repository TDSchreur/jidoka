//! How the CLI finds the dashboard: `dashboard.json` in the data folder names the port, pid and build of the
//! dashboard that wrote it. The file only counts while a Jidoka dashboard with that pid answers on that port.

use crate::{
    client::{Probe, probe_dashboard},
    server::{LOOPBACK_HOST, write_private_file},
};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

const FILE_NAME: &str = "dashboard.json";

/// What `dashboard.json` holds.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub(crate) struct Dashboard {
    pub(crate) port: u16,
    pub(crate) pid: u32,
    pub(crate) build: String,
}

/// A dashboard that answered its probe.
pub(crate) struct Running {
    pub(crate) port: u16,
    pub(crate) pid: u32,
    /// The build the running dashboard reports.
    pub(crate) version: String,
}

/// Writes `dashboard.json` into `dir`, replacing any file there; it is private to the user like the CLI token.
pub(crate) fn write(dir: &Path, dashboard: &Dashboard) -> io::Result<()> {
    let json = serde_json::to_string(dashboard).map_err(io::Error::other)?;
    write_private_file(&dir.join(FILE_NAME), &json)
}

/// The file's content, or `None` when it is missing or malformed.
pub(crate) fn read(dir: &Path) -> Option<Dashboard> {
    serde_json::from_slice(&std::fs::read(dir.join(FILE_NAME)).ok()?).ok()
}

/// Removes `dashboard.json` when it still names `pid`, so a dashboard never deletes a newer one's file.
pub(crate) fn remove_if_ours(dir: &Path, pid: u32) {
    if read(dir).is_some_and(|dashboard| dashboard.pid == pid) {
        // Best effort: a file that stays behind is stale and fails the probe.
        let _ = std::fs::remove_file(dir.join(FILE_NAME));
    }
}

/// The live dashboard the file points to. A missing, malformed or stale file gives `None`: the probe must find a
/// Jidoka dashboard with the file's pid, because a dashboard in WSL2 and one on Windows share `localhost`.
pub(crate) fn running(dir: &Path) -> Option<Running> {
    let file = read(dir)?;
    match probe_dashboard(&format!("{LOOPBACK_HOST}:{}", file.port)) {
        Ok(Probe::Running(info)) if info.pid == file.pid => Some(Running {
            port: file.port,
            pid: info.pid,
            version: info.version,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    fn entry(port: u16, pid: u32) -> Dashboard {
        Dashboard {
            port,
            pid,
            build: "dev abc".to_string(),
        }
    }

    #[test]
    fn the_file_is_written_read_and_removed_only_by_its_own_dashboard() {
        let dir = TestDir::new();
        assert_eq!(read(dir.0.path()), None);
        write(dir.0.path(), &entry(4400, 7)).unwrap();
        assert_eq!(read(dir.0.path()), Some(entry(4400, 7)));
        remove_if_ours(dir.0.path(), 8);
        assert_eq!(read(dir.0.path()), Some(entry(4400, 7)));
        remove_if_ours(dir.0.path(), 7);
        assert_eq!(read(dir.0.path()), None);
    }

    #[test]
    fn a_malformed_file_reads_as_none() {
        let dir = TestDir::new();
        std::fs::write(dir.0.path().join(FILE_NAME), "not json").unwrap();
        assert_eq!(read(dir.0.path()), None);
    }

    #[test]
    fn a_file_pointing_at_nothing_is_no_dashboard() {
        let dir = TestDir::new();
        // Port 1 is never a test server's port.
        write(dir.0.path(), &entry(1, std::process::id())).unwrap();
        assert!(running(dir.0.path()).is_none());
    }

    #[test]
    fn a_live_dashboard_with_another_pid_is_no_dashboard() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind((LOOPBACK_HOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let pid = std::process::id() + 1;
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 512];
            let _ = stream.read(&mut request).unwrap();
            let body = format!(r#"{{"pid":{pid},"version":"dev abc"}}"#);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let dir = TestDir::new();
        write(dir.0.path(), &entry(port, std::process::id())).unwrap();
        assert!(running(dir.0.path()).is_none());
        server.join().unwrap();
    }
}
