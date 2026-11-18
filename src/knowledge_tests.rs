use super::*;
use std::process::Command;

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
    let linux = |pairs: &[(&str, &str)]| file_in(env(pairs), false);
    assert_eq!(
        linux(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/me")]),
        Some(PathBuf::from("/xdg/jidoka/knowledge.json"))
    );
    assert_eq!(
        linux(&[("HOME", "/home/me")]),
        Some(PathBuf::from("/home/me/.config/jidoka/knowledge.json"))
    );
    // A relative folder is ignored, as for the model-routing policy.
    assert_eq!(
        linux(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/home/me")]),
        Some(PathBuf::from("/home/me/.config/jidoka/knowledge.json"))
    );
    assert_eq!(linux(&[]), None);
    // Windows reads APPDATA, never HOME. The path is only absolute on Windows itself.
    if cfg!(windows) {
        assert_eq!(
            file_in(env(&[("APPDATA", r"C:\Users\me\AppData\Roaming")]), true),
            Some(PathBuf::from(
                r"C:\Users\me\AppData\Roaming\jidoka\knowledge.json"
            ))
        );
    }
    assert_eq!(file_in(env(&[("HOME", "/home/me")]), true), None);
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
fn github_links_come_from_every_remote_spelling() {
    let want = "https://github.com/acme/platform/blob/main/changes/x.md";
    for remote in [
        "https://github.com/acme/platform",
        "https://github.com/acme/platform.git",
        "https://user@github.com/acme/platform.git/",
        "git@github.com:acme/platform.git",
        "ssh://git@github.com/acme/platform.git",
        "ssh://git@github.com:22/acme/platform",
    ] {
        assert_eq!(
            web_link(remote, "main", "changes/x.md").unwrap(),
            want,
            "{remote}"
        );
    }
}

#[test]
fn azure_links_come_from_every_remote_spelling() {
    let want =
        "https://dev.azure.com/org/My%20Project/_git/platform?path=/changes/x.md&version=GBmain";
    for remote in [
        "https://dev.azure.com/org/My%20Project/_git/platform",
        "https://org@dev.azure.com/org/My%20Project/_git/platform",
        "git@ssh.dev.azure.com:v3/org/My%20Project/platform",
        "https://org.visualstudio.com/My%20Project/_git/platform",
        "https://org.visualstudio.com/DefaultCollection/My%20Project/_git/platform",
    ] {
        assert_eq!(
            web_link(remote, "main", "changes/x.md").unwrap(),
            want,
            "{remote}"
        );
    }
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

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
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
    assert_eq!(spec_link(spec.to_str().unwrap(), &knowledge).unwrap(), want);
    assert_eq!(spec_link("changes/x.md", &knowledge).unwrap(), want);
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
        spec_link("changes/x.md", &knowledge).unwrap().as_deref(),
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
    let error = spec_link("changes/x.md", &knowledge).unwrap_err();
    assert!(error.contains("not committed"), "{error}");

    git(&checkout, &["add", "."]);
    let error = spec_link("changes/x.md", &knowledge).unwrap_err();
    assert!(error.contains("changes that are not committed"), "{error}");

    git(&checkout, &["commit", "-q", "-m", "spec"]);
    let error = spec_link("changes/x.md", &knowledge).unwrap_err();
    assert!(error.contains("not pushed to origin/main"), "{error}");

    git(&checkout, &["push", "-q"]);
    assert!(spec_link("changes/x.md", &knowledge).unwrap().is_some());

    fs::write(&spec, "edited").unwrap();
    assert!(
        spec_link("changes/x.md", &knowledge)
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
        spec_link(other.to_str().unwrap(), &Knowledge::default()).unwrap(),
        None
    );

    let (_, knowledge) = knowledge_repo(base.path(), "https://github.com/acme/platform");
    assert_eq!(
        spec_link(other.to_str().unwrap(), &knowledge).unwrap(),
        None
    );
    assert_eq!(spec_link("changes/missing.md", &knowledge).unwrap(), None);
}

#[test]
fn guidance_needs_a_configured_knowledge_repo() {
    assert_eq!(Guidance::for_setting(&Knowledge::default()), None);
}
