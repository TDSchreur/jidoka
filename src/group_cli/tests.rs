use super::*;
use crate::pr_state::{PrState, PrStatus};

fn member(pr: Option<&str>, status: Option<PrStatus>) -> Member {
    Member {
        run: "1".into(),
        missing: false,
        title: Some("task".into()),
        repository: Some("/work/api".into()),
        pr: pr.map(String::from),
        pr_status: status,
    }
}

#[test]
fn create_options_need_a_spec_and_accept_repeated_runs() {
    let (spec, runs) =
        parse_create(&["--spec", "https://x.test/s", "--run", "1", "--run", "2"]).unwrap();
    assert_eq!(spec, "https://x.test/s");
    assert_eq!(runs, ["1", "2"]);
    assert!(parse_create(&["--run", "1"]).is_none());
    assert!(parse_create(&["--spec"]).is_none());
    assert!(parse_create(&["--spec", "a", "--spec", "b"]).is_none());
    assert!(parse_create(&["--bogus", "a"]).is_none());
}

#[test]
fn a_local_spec_is_stored_as_an_absolute_path_and_a_url_as_given() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("spec.md");
    std::fs::write(&file, "x").unwrap();
    let stored = spec_reference(file.to_str().unwrap()).unwrap();
    assert!(Path::new(&stored).is_absolute());
    assert_eq!(
        spec_reference("https://x.test/s").unwrap(),
        "https://x.test/s"
    );
    assert!(spec_reference(dir.path().join("missing.md").to_str().unwrap()).is_err());
}

#[test]
fn a_pull_request_is_missing_unchecked_or_shown_with_its_state() {
    let now = 10 * 60 * 1000;
    assert_eq!(pull_request(&member(None, None), now), "none recorded");
    assert_eq!(
        pull_request(&member(Some("https://x.test/1"), None), now),
        "https://x.test/1 (unchecked)"
    );
    let status = PrStatus {
        url: "https://x.test/1".into(),
        state: PrState::Merged,
        reason: None,
        checked_at: 0,
    };
    assert_eq!(
        pull_request(&member(Some("https://x.test/1"), Some(status)), now),
        "https://x.test/1 (merged, checked 10 min ago)"
    );
}

#[test]
fn a_missing_run_says_how_to_detach_it() {
    let missing = Member {
        run: "7".into(),
        missing: true,
        title: None,
        repository: None,
        pr: None,
        pr_status: None,
    };
    assert!(render_member(&missing, 0).contains("run missing"));
}
