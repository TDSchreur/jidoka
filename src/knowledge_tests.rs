use super::*;
use crate::git::{
    Git,
    fixture::{TestGit, run_git as git},
};

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    }
}

#[test]
fn the_setting_lives_in_the_config_folder_of_each_platform() {
    // Absolute on the platform the test runs on; the Linux rules are checked everywhere, as they only read variables.
    let root = if cfg!(windows) { r"C:\base" } else { "/base" };
    let xdg = format!("{root}{}xdg", std::path::MAIN_SEPARATOR);
    let home = format!("{root}{}home", std::path::MAIN_SEPARATOR);
    let linux = |pairs: &[(&str, &str)]| file_in(env(pairs), false);
    let json = |base: &str, folder: &[&str]| {
        let mut path = PathBuf::from(base);
        path.extend(folder);
        Some(path.join("jidoka").join("knowledge.json"))
    };
    assert_eq!(
        linux(&[("XDG_CONFIG_HOME", &xdg), ("HOME", &home)]),
        json(&xdg, &[])
    );
    assert_eq!(linux(&[("HOME", &home)]), json(&home, &[".config"]));
    // A relative folder is ignored, as for the model-routing policy.
    assert_eq!(
        linux(&[("XDG_CONFIG_HOME", "rel"), ("HOME", &home)]),
        json(&home, &[".config"])
    );
    assert_eq!(linux(&[]), None);
    // Windows reads APPDATA, never HOME.
    assert_eq!(
        file_in(env(&[("APPDATA", &xdg), ("HOME", &home)]), true),
        json(&xdg, &[])
    );
    assert_eq!(file_in(env(&[("HOME", &home)]), true), None);
}

#[test]
fn the_setting_round_trips_and_a_missing_file_is_nothing_configured() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("jidoka").join("knowledge.json");
    assert_eq!(read(&file).unwrap(), Knowledge::default());
    let knowledge = Knowledge {
        path: Some("/work/knowledge".into()),
        checkouts: BTreeMap::from([("infra".into(), "/work/infra".into())]),
    };
    write(&file, &knowledge).unwrap();
    assert_eq!(read(&file).unwrap(), knowledge);
    fs::write(&file, "not json").unwrap();
    assert!(
        read(&file)
            .unwrap_err()
            .contains("not a valid knowledge setting")
    );
}

#[test]
fn links_come_from_the_remote_of_either_host() {
    // Every remote spelling of each host is covered by the pull request host's tests.
    assert_eq!(
        web_link("git@github.com:acme/platform.git", "main", "changes/x.md").unwrap(),
        "https://github.com/acme/platform/blob/main/changes/x.md"
    );
    assert_eq!(
        web_link(
            "https://org.visualstudio.com/My%20Project/_git/platform",
            "main",
            "changes/x.md"
        )
        .unwrap(),
        "https://dev.azure.com/org/My%20Project/_git/platform?path=/changes/x.md&version=GBmain"
    );
}

#[test]
fn branches_and_file_names_are_encoded() {
    assert_eq!(
        web_link("https://github.com/a/b", "release/1", "changes/my spec.md").unwrap(),
        "https://github.com/a/b/blob/release/1/changes/my%20spec.md"
    );
    assert_eq!(
        web_link(
            "https://dev.azure.com/o/p/_git/r",
            "release/1",
            "changes/my spec.md"
        )
        .unwrap(),
        "https://dev.azure.com/o/p/_git/r?path=/changes/my%20spec.md&version=GBrelease%2F1"
    );
}

#[test]
fn another_host_has_no_link() {
    for remote in ["https://gitlab.com/a/b", "/srv/git/b.git", r"C:\git\b"] {
        let error = web_link(remote, "main", "x.md").unwrap_err();
        assert!(
            error.contains("GitHub or Azure DevOps"),
            "{remote}: {error}"
        );
    }
}

/// A knowledge repo checkout whose origin is `remote` for reading and a local bare repository for pushing.
fn knowledge_repo(base: &Path, remote: &str) -> (PathBuf, Knowledge) {
    let bare = base.join("origin.git");
    fs::create_dir(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    let checkout = base.join("knowledge");
    fs::create_dir(&checkout).unwrap();
    git(&checkout, &["init", "-q", "-b", "main"]);
    git(&checkout, &["remote", "add", "origin", remote]);
    git(
        &checkout,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            bare.to_str().unwrap(),
        ],
    );
    fs::create_dir(checkout.join("changes")).unwrap();
    let knowledge = Knowledge {
        path: Some(checkout.to_string_lossy().into_owned()),
        ..Default::default()
    };
    (checkout, knowledge)
}

#[test]
fn a_pushed_spec_resolves_by_relative_or_absolute_path() {
    let base = tempfile::tempdir().unwrap();
    let (checkout, knowledge) = knowledge_repo(base.path(), "https://github.com/acme/platform.git");
    let spec = checkout.join("changes").join("x.md");
    fs::write(&spec, "spec").unwrap();
    git(&checkout, &["add", "."]);
    git(&checkout, &["commit", "-q", "-m", "spec"]);
    git(&checkout, &["push", "-q", "-u", "origin", "main"]);

    let want = Some("https://github.com/acme/platform/blob/main/changes/x.md".to_string());
    assert_eq!(
        spec_link(spec.to_str().unwrap(), &knowledge, &TestGit::default()).unwrap(),
        want
    );
    assert_eq!(
        spec_link("changes/x.md", &knowledge, &TestGit::default()).unwrap(),
        want
    );
}

#[test]
fn an_azure_origin_resolves_the_same_way() {
    let base = tempfile::tempdir().unwrap();
    let (checkout, knowledge) =
        knowledge_repo(base.path(), "git@ssh.dev.azure.com:v3/org/proj/platform");
    fs::write(checkout.join("changes").join("x.md"), "spec").unwrap();
    git(&checkout, &["add", "."]);
    git(&checkout, &["commit", "-q", "-m", "spec"]);
    git(&checkout, &["push", "-q", "-u", "origin", "main"]);
    assert_eq!(
        spec_link("changes/x.md", &knowledge, &TestGit::default())
            .unwrap()
            .as_deref(),
        Some("https://dev.azure.com/org/proj/_git/platform?path=/changes/x.md&version=GBmain")
    );
}

#[test]
fn an_unpushed_spec_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let (checkout, knowledge) = knowledge_repo(base.path(), "https://github.com/acme/platform");
    fs::write(checkout.join("README.md"), "readme").unwrap();
    git(&checkout, &["add", "."]);
    git(&checkout, &["commit", "-q", "-m", "first"]);
    git(&checkout, &["push", "-q", "-u", "origin", "main"]);

    let spec = checkout.join("changes").join("x.md");
    fs::write(&spec, "spec").unwrap();
    let error = spec_link("changes/x.md", &knowledge, &TestGit::default()).unwrap_err();
    assert!(error.contains("not committed"), "{error}");

    git(&checkout, &["add", "."]);
    let error = spec_link("changes/x.md", &knowledge, &TestGit::default()).unwrap_err();
    assert!(error.contains("changes that are not committed"), "{error}");

    git(&checkout, &["commit", "-q", "-m", "spec"]);
    let error = spec_link("changes/x.md", &knowledge, &TestGit::default()).unwrap_err();
    assert!(error.contains("not pushed to origin/main"), "{error}");

    git(&checkout, &["push", "-q"]);
    assert!(
        spec_link("changes/x.md", &knowledge, &TestGit::default())
            .unwrap()
            .is_some()
    );

    fs::write(&spec, "edited").unwrap();
    assert!(
        spec_link("changes/x.md", &knowledge, &TestGit::default())
            .unwrap_err()
            .contains("not committed")
    );
}

#[test]
fn without_a_knowledge_repo_or_for_other_files_nothing_changes() {
    let base = tempfile::tempdir().unwrap();
    let other = base.path().join("spec.md");
    fs::write(&other, "spec").unwrap();
    assert_eq!(
        spec_link(
            other.to_str().unwrap(),
            &Knowledge::default(),
            &TestGit::default()
        )
        .unwrap(),
        None
    );

    let (_, knowledge) = knowledge_repo(base.path(), "https://github.com/acme/platform");
    assert_eq!(
        spec_link(other.to_str().unwrap(), &knowledge, &TestGit::default()).unwrap(),
        None
    );
    assert_eq!(
        spec_link("changes/missing.md", &knowledge, &TestGit::default()).unwrap(),
        None
    );
}

/// A knowledge repo whose origin does not name its default branch and whose checkout is on `trunk`, with
/// every spec committed and pushed.
struct TrunkGit;

impl Git for TrunkGit {
    fn git(&self, _: &Path, args: &[&str]) -> Result<String, String> {
        match args {
            ["remote", "get-url", "origin"] => Ok("https://github.com/acme/platform.git".into()),
            ["symbolic-ref", ..] => {
                Err("fatal: ref refs/remotes/origin/HEAD is not a symbolic ref".into())
            }
            ["rev-parse", "--abbrev-ref", "HEAD"] => Ok("trunk".into()),
            ["log", ..] => Ok("abc".into()),
            ["ls-files" | "status" | "rev-parse" | "merge-base", ..] => Ok(String::new()),
            _ => panic!("unexpected git {args:?}"),
        }
    }
}

#[test]
fn without_a_default_branch_on_origin_the_checked_out_branch_is_linked() {
    let base = tempfile::tempdir().unwrap();
    fs::create_dir(base.path().join("changes")).unwrap();
    fs::write(base.path().join("changes").join("x.md"), "spec").unwrap();
    let knowledge = Knowledge {
        path: Some(base.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    assert_eq!(
        spec_link("changes/x.md", &knowledge, &TrunkGit)
            .unwrap()
            .as_deref(),
        Some("https://github.com/acme/platform/blob/trunk/changes/x.md")
    );
}

#[test]
fn guidance_needs_a_configured_knowledge_repo() {
    assert_eq!(Guidance::for_setting(&Knowledge::default()), None);
}
