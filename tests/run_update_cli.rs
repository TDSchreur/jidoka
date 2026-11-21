//! The CLI refuses lifecycle arguments to metadata reporting before contacting the dashboard.

mod common;

#[test]
fn update_refuses_positional_status_and_stage_arguments() {
    let data = tempfile::tempdir().unwrap();
    for argument in ["running", "blocked", "done", "finish"] {
        let output = common::jidoka_in(data.path())
            .args(["run", "update", "1", argument])
            .env("XDG_DATA_HOME", data.path())
            .env("LOCALAPPDATA", data.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!("Unknown run update option: {argument}")),
            "{stderr}"
        );
    }
}
