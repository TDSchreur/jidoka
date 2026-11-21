//! The per-user knowledge repo setting: `jidoka knowledge`, the web link of a spec stored in the knowledge
//! repo, and the guidance `jidoka run next` gives when the setting exists.
//!
//! The setting is `knowledge.json` in Jidoka's configuration folder (see ADR-0015). It is per user, not per
//! project, because a platform has one knowledge repo and checkout paths differ per machine.

use crate::pr_host::{AzureRepo, encode_segment};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

type Failure = Box<dyn std::error::Error>;

/// What `knowledge.json` holds.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq)]
pub(crate) struct Knowledge {
    /// The knowledge repo checkout on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    /// Repository name to its checkout on this machine. Never shared: the knowledge repo lists no paths.
    #[serde(default)]
    pub(crate) checkouts: BTreeMap<String, String>,
}

/// Where the setting lives for the machine described by `var` and `windows`.
pub(crate) fn file_in(var: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    Some(crate::routing::config_folder_in(var, windows)?.join("knowledge.json"))
}

fn file() -> Result<PathBuf, String> {
    file_in(|name| std::env::var_os(name), cfg!(windows)).ok_or_else(|| {
        if cfg!(windows) {
            "cannot work out the configuration folder; set APPDATA"
        } else {
            "cannot work out the configuration folder; set XDG_CONFIG_HOME or HOME"
        }
        .to_string()
    })
}

/// Reads the setting; no file means nothing is configured.
pub(crate) fn read(path: &Path) -> Result<Knowledge, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| format!("{}: not a valid knowledge setting: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Knowledge::default()),
        Err(error) => Err(format!(
            "{}: cannot read the setting: {error}",
            path.display()
        )),
    }
}

fn write(path: &Path, knowledge: &Knowledge) -> Result<(), String> {
    let fail = |error: io::Error| format!("{}: cannot write the setting: {error}", path.display());
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(fail)?;
    }
    let text = serde_json::to_string_pretty(knowledge).map_err(|error| error.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(fail)
}

/// The setting of this machine's user.
pub(crate) fn load() -> Result<Knowledge, String> {
    read(&file()?)
}

/// `jidoka knowledge ...`; `words` are the arguments after `knowledge`.
pub(crate) fn run(words: &[&str]) -> Result<(), Failure> {
    let usage = || "Invalid arguments; see `jidoka --help`".to_string();
    let file = file()?;
    match words {
        [] | ["show"] => {
            let knowledge = read(&file)?;
            println!("Setting  {}", file.display());
            match &knowledge.path {
                Some(path) => println!("Knowledge repo  {path}"),
                None => println!("Knowledge repo  not set (jidoka knowledge set <path>)"),
            }
            for (name, path) in &knowledge.checkouts {
                println!("  {name}  {path}");
            }
        }
        ["set", path] => {
            let absolute = std::path::absolute(path)?;
            if !absolute.join(".git").exists() {
                return Err(format!("{} is not a git checkout", absolute.display()).into());
            }
            let mut knowledge = read(&file)?;
            knowledge.path = Some(absolute.to_string_lossy().into_owned());
            write(&file, &knowledge)?;
            println!("Knowledge repo is now {}", absolute.display());
        }
        ["checkout", name, path] if !name.is_empty() => {
            let absolute = std::path::absolute(path)?;
            if !absolute.is_dir() {
                return Err(format!("{} is not a folder", absolute.display()).into());
            }
            let mut knowledge = read(&file)?;
            knowledge
                .checkouts
                .insert((*name).to_string(), absolute.to_string_lossy().into_owned());
            write(&file, &knowledge)?;
            println!("Checkout of {name} is now {}", absolute.display());
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

/// The web link of `spec` when it is a file of the configured knowledge repo, `None` when the setting does
/// not apply (nothing configured, or the file is elsewhere) so the caller treats it as before.
///
/// # Errors
///
/// A message when the file is in the knowledge repo but cannot be linked: no usable origin, an unsupported
/// host, or the file is not pushed.
pub(crate) fn spec_link(spec: &str, knowledge: &Knowledge) -> Result<Option<String>, String> {
    let Some(root) = &knowledge.path else {
        return Ok(None);
    };
    let root = fs::canonicalize(root).map_err(|error| {
        format!("the knowledge repo checkout {root} is not usable ({error}); fix it with `jidoka knowledge set <path>`")
    })?;
    let candidates = [
        std::path::absolute(spec).ok(),
        Path::new(spec).is_relative().then_some(root.join(spec)),
    ];
    let Some(file) = candidates
        .into_iter()
        .flatten()
        .filter_map(|candidate| fs::canonicalize(candidate).ok())
        .find(|candidate| candidate.is_file() && candidate.starts_with(&root))
    else {
        return Ok(None);
    };
    let relative: Vec<String> = file
        .strip_prefix(&root)
        .map_err(|error| error.to_string())?
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let relative = relative.join("/");

    let remote = git(&root, &["remote", "get-url", "origin"])
        .map_err(|_| "the knowledge repo has no origin remote".to_string())?;
    let branch = default_branch(&root)?;
    ensure_pushed(&root, &relative, &branch)?;
    web_link(&remote, &branch, &relative).map(Some)
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// The remote's default branch, or the checked out one when origin does not say.
fn default_branch(root: &Path) -> Result<String, String> {
    if let Ok(head) = git(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) && let Some(branch) = head.strip_prefix("origin/")
    {
        return Ok(branch.to_string());
    }
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map_err(|error| format!("cannot find the knowledge repo's default branch: {error}"))
}

/// Refuses a spec that reviewers could not open: untracked, changed since its commit, or not on origin.
fn ensure_pushed(root: &Path, relative: &str, branch: &str) -> Result<(), String> {
    let hint = "commit it and push it to the default branch first";
    if git(root, &["ls-files", "--error-unmatch", "--", relative]).is_err() {
        return Err(format!(
            "{relative} is not committed in the knowledge repo; {hint}"
        ));
    }
    if !git(root, &["status", "--porcelain", "--", relative])?.is_empty() {
        return Err(format!(
            "{relative} has changes that are not committed; {hint}"
        ));
    }
    let remote_branch = format!("refs/remotes/origin/{branch}");
    git(root, &["rev-parse", "--verify", "--quiet", &remote_branch])
        .map_err(|_| format!("origin/{branch} is unknown in the knowledge repo; {hint}"))?;
    let commit = git(root, &["log", "-1", "--format=%H", "--", relative])?;
    if git(
        root,
        &["merge-base", "--is-ancestor", &commit, &remote_branch],
    )
    .is_err()
    {
        return Err(format!(
            "{relative} is not pushed to origin/{branch}; {hint}"
        ));
    }
    Ok(())
}

/// The web link of `relative` (with `/` separators) on `branch` of the repository at `remote`.
fn web_link(remote: &str, branch: &str, relative: &str) -> Result<String, String> {
    let path = |separator: &str| {
        relative
            .split('/')
            .map(encode_segment)
            .collect::<Vec<_>>()
            .join(separator)
    };
    if let Some(azure) = AzureRepo::from_remote(remote).map_err(|problem| problem.0)? {
        return Ok(format!(
            "https://dev.azure.com/{}/{}/_git/{}?path=/{}&version=GB{}",
            encode_segment(&azure.organization),
            encode_segment(&azure.project),
            encode_segment(&azure.repository),
            path("/"),
            encode_segment(branch)
        ));
    }
    let (owner, repository) = github_repo(remote).ok_or_else(|| {
        "the knowledge repo's origin is not a GitHub or Azure DevOps remote, so its files have no web link"
            .to_string()
    })?;
    let branch = branch
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    Ok(format!(
        "https://github.com/{}/{}/blob/{branch}/{}",
        encode_segment(&owner),
        encode_segment(&repository),
        path("/")
    ))
}

/// Owner and repository of a github.com remote in any of its usual spellings.
fn github_repo(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim_end_matches('/');
    let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
        path
    } else {
        let (_, rest) = remote.split_once("://")?;
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        host.eq_ignore_ascii_case("github.com").then_some(path)?
    };
    let path = path.strip_suffix(".git").unwrap_or(path);
    match path.split('/').collect::<Vec<_>>().as_slice() {
        [owner, repository] if !owner.is_empty() && !repository.is_empty() => {
            Some(((*owner).to_string(), (*repository).to_string()))
        }
        _ => None,
    }
}

/// What `jidoka run next` adds to Define and Finish when a knowledge repo is configured.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct Guidance {
    pub(crate) path: String,
    pub(crate) instructions: String,
}

impl Guidance {
    /// The guidance for `knowledge`, `None` when no knowledge repo is configured.
    pub(crate) fn for_setting(knowledge: &Knowledge) -> Option<Self> {
        let path = knowledge.path.clone()?;
        let instructions =
            crate::install::bundled_skill_text("jidoka-procedures/chunks/knowledge-repo.md")
                .expect("knowledge-repo chunk is bundled")
                .to_string();
        Some(Self { path, instructions })
    }
}

#[cfg(test)]
#[path = "knowledge_tests.rs"]
mod tests;
