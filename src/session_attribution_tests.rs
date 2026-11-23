//! Local Claude Code log fixtures read through the shared operation: which sessions a Run claims, which
//! responses are its own or ambiguous, and why a scan found nothing. Nothing here reads a real user's logs.

use super::*;
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};
use tempfile::TempDir;

/// 2026-11-05T10:00:00Z.
const T0: u64 = 1_793_872_800_000;
const RUN_A: &str = "1793872800000000001";
const RUN_B: &str = "1793872800000000002";
/// Long after every fixture, so no response is in the future.
const NOW: u64 = T0 + 10_000_000;

fn at(seconds: u64) -> u64 {
    T0 + seconds * 1000
}

fn run(id: &str, start: u64, end: u64, sessions: &[&str]) -> Run {
    serde_json::from_value(json!({
        "id": id, "title": "t", "gates": [], "project": "/p", "stage": "build", "status": "running",
        "timeline": [{"at": at(start), "stage": "build", "rework": null, "ticket": null}],
        "finished_at": at(end), "sessions": sessions,
    }))
    .unwrap()
}

/// A Claude Code response at `seconds` after T0; with `claims`, its Bash command names that run.
fn response(seconds: u64, id: &str, claims: Option<&str>) -> String {
    let block: Value = match claims {
        Some(run) => json!({"type": "tool_use", "id": "t", "name": "Bash",
            "input": {"command": format!("jidoka run update {run} build running")}}),
        None => json!({"type": "text", "text": "done"}),
    };
    json!({"type": "assistant", "timestamp": crate::usage::iso_timestamp(at(seconds)), "requestId": format!("req-{id}"),
        "isSidechain": false, "message": {"id": id, "model": "claude-test", "role": "assistant", "content": [block],
        "usage": {"input_tokens": 1, "output_tokens": 1}}})
    .to_string()
}

fn write_session(home: &Path, session: &str, lines: &[String]) {
    let path = home.join(format!("projects/slug/{session}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, lines.join("\n") + "\n").unwrap();
}

/// The log folders of an environment whose Claude Code home is `home`.
fn roots(home: &Path) -> Roots {
    Roots {
        claude: home.join("projects"),
        codex: home.join("codex/sessions"),
        copilot: home.join("copilot/session-state"),
    }
}

fn scan_of<'a>(target: &'a Run, runs: &'a [Run], home: &Path) -> SessionScan<'a> {
    scan_limited(target, runs, home, ReadLimits::default())
}

fn scan_limited<'a>(
    target: &'a Run,
    runs: &'a [Run],
    home: &Path,
    limits: ReadLimits,
) -> SessionScan<'a> {
    let deadline = Instant::now() + Duration::from_secs(30);
    scan(target, runs, &roots(home), deadline, limits)
        .unwrap_or_else(|reason| panic!("the run has a timeline: {reason}"))
}

fn ids(sessions: &[&LogSession]) -> Vec<String> {
    sessions.iter().map(|session| session.id.clone()).collect()
}

fn times(attribution: &Attribution) -> Vec<u64> {
    attribution
        .responses
        .iter()
        .map(|attributed| attributed.response.ts)
        .collect()
}

#[test]
fn a_run_claims_recorded_sessions_and_sessions_whose_commands_name_it() {
    let home = TempDir::new().unwrap();
    write_session(home.path(), "recorded", &[response(10, "m1", None)]);
    write_session(home.path(), "inferred", &[response(20, "m2", Some(RUN_A))]);
    write_session(home.path(), "other", &[response(30, "m3", None)]);
    let target = run(RUN_A, 0, 100, &["recorded"]);
    let runs = std::slice::from_ref(&target);

    let scan = scan_of(&target, runs, home.path());
    assert_eq!(ids(&scan.claimed()), ["inferred", "recorded"]);
    assert_eq!(scan.missing(), None);
    let attribution = scan.attribute(NOW, None);
    assert_eq!(times(&attribution), [at(20), at(10)]);
    assert_eq!(ids(&attribution.sessions), ["inferred", "recorded"]);
    assert!(attribution.ambiguous.is_empty());
}

#[test]
fn a_response_both_overlapping_runs_can_claim_is_ambiguous_and_the_others_keep_their_marks() {
    let home = TempDir::new().unwrap();
    // One session names both runs; B starts while A is still working.
    write_session(
        home.path(),
        "shared",
        &[
            response(10, "m1", Some(RUN_A)),
            response(60, "m2", Some(RUN_B)),
            response(150, "m3", None),
        ],
    );
    let a = run(RUN_A, 0, 100, &[]);
    let b = run(RUN_B, 50, 200, &[]);
    let runs = [a.clone(), b.clone()];

    let for_a = scan_of(&a, &runs, home.path());
    let attribution = for_a.attribute(NOW, None);
    assert_eq!(times(&attribution), [at(10)]);
    assert_eq!(attribution.responses[0].mark.at, at(0));
    let ambiguous: Vec<u64> = attribution.ambiguous.iter().map(|r| r.ts).collect();
    assert_eq!(ambiguous, [at(60)]);

    let for_b = scan_of(&b, &runs, home.path());
    let attribution = for_b.attribute(NOW, None);
    assert_eq!(times(&attribution), [at(150)]);
    assert_eq!(attribution.ambiguous.len(), 1);
}

#[test]
fn an_overlapping_run_that_does_not_claim_the_session_does_not_make_it_ambiguous() {
    let home = TempDir::new().unwrap();
    write_session(home.path(), "a", &[response(60, "m1", Some(RUN_A))]);
    let a = run(RUN_A, 0, 100, &[]);
    let runs = [a.clone(), run(RUN_B, 50, 200, &["b"])];

    let scan = scan_of(&a, &runs, home.path());
    let attribution = scan.attribute(NOW, None);
    assert_eq!(times(&attribution), [at(60)]);
    assert!(attribution.ambiguous.is_empty());
}

#[test]
fn a_run_that_claims_the_session_at_another_time_does_not_make_it_ambiguous() {
    let home = TempDir::new().unwrap();
    write_session(
        home.path(),
        "shared",
        &[
            response(10, "m1", Some(RUN_A)),
            response(300, "m2", Some(RUN_B)),
        ],
    );
    let a = run(RUN_A, 0, 100, &[]);
    let runs = [a.clone(), run(RUN_B, 200, 400, &[])];

    let scan = scan_of(&a, &runs, home.path());
    let attribution = scan.attribute(NOW, None);
    assert_eq!(times(&attribution), [at(10)]);
    assert!(attribution.ambiguous.is_empty());
}

#[test]
fn a_lead_in_counts_responses_shortly_before_the_first_mark_under_that_mark() {
    let home = TempDir::new().unwrap();
    write_session(
        home.path(),
        "s",
        &[
            response(100, "early", None),
            response(150, "lead", None),
            response(210, "late", None),
            response(300, "after", None),
        ],
    );
    let target = run(RUN_A, 200, 250, &["s"]);
    let runs = std::slice::from_ref(&target);
    let scan = scan_of(&target, runs, home.path());

    assert_eq!(times(&scan.attribute(NOW, None)), [at(210)]);
    let lead_in = LeadIn {
        ms: 60_000,
        applies: |_| true,
    };
    let attribution = scan.attribute(NOW, Some(lead_in));
    assert_eq!(times(&attribution), [at(150), at(210)]);
    assert_eq!(attribution.responses[0].mark.at, at(200));
    let none_apply = LeadIn {
        ms: 60_000,
        applies: |_| false,
    };
    assert_eq!(times(&scan.attribute(NOW, Some(none_apply))), [at(210)]);
}

#[test]
fn a_response_from_the_future_is_not_attributed() {
    let home = TempDir::new().unwrap();
    write_session(
        home.path(),
        "s",
        &[response(10, "m1", None), response(90, "m2", None)],
    );
    let target = run(RUN_A, 0, 100, &["s"]);
    let runs = std::slice::from_ref(&target);

    let scan = scan_of(&target, runs, home.path());
    let attribution = scan.attribute(at(20), None);
    assert_eq!(times(&attribution), [at(10)]);
}

#[test]
fn a_scan_cut_short_by_its_limit_says_so_and_finds_no_session() {
    let home = TempDir::new().unwrap();
    write_session(home.path(), "s", &[response(10, "m1", Some(RUN_A))]);
    let target = run(RUN_A, 0, 100, &[]);
    let runs = std::slice::from_ref(&target);
    let limits = ReadLimits { files: 0, bytes: 0 };

    let scan = scan_limited(&target, runs, home.path(), limits);
    assert!(scan.read.truncated);
    assert_eq!(scan.missing(), Some(Missing::Unclaimed));
}

#[test]
fn a_scan_with_no_log_folders_or_no_timeline_says_why() {
    let home = TempDir::new().unwrap();
    let target = run(RUN_A, 0, 100, &[]);
    let runs = std::slice::from_ref(&target);
    assert_eq!(
        scan_of(&target, runs, home.path()).missing(),
        Some(Missing::NoLogs)
    );

    let mut untimed = target.clone();
    untimed.timeline.clear();
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = scan(
        &untimed,
        std::slice::from_ref(&untimed),
        &roots(home.path()),
        deadline,
        ReadLimits::default(),
    );
    assert_eq!(result.err(), Some(NO_TIMELINE));
}

#[test]
fn a_run_is_scanned_in_the_agent_home_it_recorded_and_a_missing_session_names_that_home() {
    let environment = TempDir::new().unwrap();
    let recorded = TempDir::new().unwrap();
    write_session(recorded.path(), "s", &[response(10, "m1", None)]);
    let mut target = run(RUN_A, 0, 100, &["s"]);
    let runs = std::slice::from_ref(&target);
    let Some(Missing::RecordedNotFound(note)) =
        scan_of(&target, runs, environment.path()).missing()
    else {
        panic!("the recorded session is not in the environment's home");
    };
    assert!(note.starts_with(RECORDED_NOT_FOUND), "{note}");
    assert!(
        note.contains(&environment.path().display().to_string()),
        "{note}"
    );

    target
        .agent_homes
        .insert("claude".into(), recorded.path().to_str().unwrap().into());
    let runs = std::slice::from_ref(&target);
    let scan = scan_of(&target, runs, environment.path());
    assert_eq!(ids(&scan.claimed()), ["s"]);
    assert_eq!(times(&scan.attribute(NOW, None)), [at(10)]);

    target.sessions = vec!["gone".into()];
    let runs = std::slice::from_ref(&target);
    let Some(Missing::RecordedNotFound(note)) =
        scan_of(&target, runs, environment.path()).missing()
    else {
        panic!("the recorded session is in neither home");
    };
    assert!(
        note.contains(&format!("scanned folders: {},", recorded.path().display())),
        "{note}"
    );
}
