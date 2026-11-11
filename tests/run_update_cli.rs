//! `jidoka run update` takes a status only: a stage argument is refused before any request.

mod common;

#[test]
fn update_refuses_a_stage_argument() {
    let data = tempfile::tempdir().unwrap();
    let output = common::jidoka_in(data.path())
        .args(["run", "update", "1", "finish", "done"])
        .env("XDG_DATA_HOME", data.path())
        .env("LOCALAPPDATA", data.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("expected one of `running`"), "{stderr}");
}
