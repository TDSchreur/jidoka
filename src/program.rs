//! Finding a command line program on `PATH` the way the operating system would start it, for `jidoka doctor`
//! and for the pull request host's tools.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

/// Finds `name` in the `PATH` folders. On Windows a name without a `PATHEXT` extension is tried with each
/// extension, and a name that already ends in one, such as `npm.cmd`, is tried as given first.
pub(crate) fn find_program(name: &str, path: &OsString) -> Option<PathBuf> {
    let extensions = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        windows_extensions(&pathext)
    } else {
        vec![String::new()]
    };
    find_with_extensions(name, path, &extensions)
}

/// What to run for `az`. Windows starts only `az.exe` for a bare name, but the Azure CLI installs `az.cmd`,
/// so the launcher found on `PATH` is used.
pub(crate) fn az_program() -> PathBuf {
    std::env::var_os("PATH").map_or_else(|| PathBuf::from("az"), |path| az_program_in(&path))
}

fn az_program_in(path: &OsString) -> PathBuf {
    find_program("az", path)
        .filter(|found| found.is_absolute())
        .unwrap_or_else(|| PathBuf::from("az"))
}

/// The extensions Windows runs, in `PATHEXT` order. A file without one, such as the Bash launcher
/// next to `az.cmd`, is not something Windows can run, so it is not a candidate.
fn windows_extensions(pathext: &str) -> Vec<String> {
    pathext
        .split(';')
        .filter(|ext| ext.starts_with('.'))
        .map(str::to_string)
        .collect()
}

/// The suffixes to append to `name`: the extensions, preceded by the empty suffix when `name` already
/// ends in one of them (compared case-insensitively).
fn candidate_suffixes(name: &str, extensions: &[String]) -> Vec<String> {
    let lower = name.to_lowercase();
    let has_extension = extensions
        .iter()
        .any(|ext| !ext.is_empty() && lower.ends_with(&ext.to_lowercase()));
    let mut suffixes = Vec::new();
    if has_extension {
        suffixes.push(String::new());
    }
    suffixes.extend(extensions.iter().cloned());
    suffixes
}

fn find_with_extensions(name: &str, path: &OsString, extensions: &[String]) -> Option<PathBuf> {
    let suffixes = candidate_suffixes(name, extensions);
    std::env::split_paths(path).find_map(|dir| {
        suffixes.iter().find_map(|suffix| {
            let candidate = dir.join(format!("{name}{suffix}"));
            is_executable(&candidate).then_some(candidate)
        })
    })
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}
