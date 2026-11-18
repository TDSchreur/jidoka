use super::*;
use crate::{run::NewRun, test_support::TestDir};

struct Fixture {
    dir: TestDir,
    runs: RunStore,
    groups: GroupStore,
}

fn fixture() -> Fixture {
    let dir = TestDir::new();
    let path = dir.0.path().join("runs.json");
    Fixture {
        runs: RunStore::open(&path).unwrap(),
        groups: GroupStore::open(&path).unwrap(),
        dir,
    }
}

impl Fixture {
    fn run(&self) -> String {
        self.runs
            .create(NewRun {
                title: "task".into(),
                project: "/work".into(),
                repository: Some("/work".into()),
                session: None,
                model_routing: None,
            })
            .unwrap()
            .id
    }

    fn group(&self, runs: &[&str]) -> Group {
        self.groups
            .create(
                &self.runs,
                NewGroup {
                    title: "platform".into(),
                    spec: "https://example.com/spec".into(),
                    runs: runs.iter().map(|id| (*id).to_string()).collect(),
                },
            )
            .unwrap()
    }

    /// The groups as the next dashboard start would load them.
    fn stored(&self) -> Vec<Group> {
        GroupStore::open(&self.dir.0.path().join("runs.json"))
            .unwrap()
            .list()
            .unwrap()
    }
}

#[test]
fn a_group_keeps_its_members_across_a_restart() {
    let fx = fixture();
    let (a, b) = (fx.run(), fx.run());
    let group = fx.group(&[&a, &b, &a]);
    assert!(valid_group_id(&group.id));
    assert_eq!(group.runs, [a.clone(), b.clone()]);
    assert_eq!(fx.stored(), [group]);
}

#[test]
fn a_run_joins_only_one_group_and_must_exist() {
    let fx = fixture();
    let run = fx.run();
    let first = fx.group(&[&run]);
    let second = fx.group(&[]);
    let taken = fx.groups.add(&fx.runs, &second.id, &run);
    assert!(matches!(taken, Err(GroupError::Conflict(m)) if m.contains(&first.id)));
    let missing = fx.groups.add(&fx.runs, &second.id, "123");
    assert!(matches!(missing, Err(GroupError::NotFound(_))));
    let bad = fx.groups.create(
        &fx.runs,
        NewGroup {
            title: "t".into(),
            spec: "https://example.com".into(),
            runs: vec![run, "123".into()],
        },
    );
    assert!(bad.is_err());
    // A create that fails on one member saves nothing.
    assert_eq!(fx.stored().len(), 2);
}

#[test]
fn a_title_and_a_spec_are_required() {
    let fx = fixture();
    for (title, spec) in [(" ", "https://example.com"), ("t", ""), ("t", "ftp://x")] {
        let result = fx.groups.create(
            &fx.runs,
            NewGroup {
                title: title.into(),
                spec: spec.into(),
                runs: vec![],
            },
        );
        assert!(
            matches!(result, Err(GroupError::Invalid(_))),
            "{title:?} {spec:?}"
        );
    }
}

#[test]
fn concurrent_attachments_of_one_run_to_two_groups_let_exactly_one_win() {
    for _ in 0..20 {
        let fx = fixture();
        let run = fx.run();
        let (first, second) = (fx.group(&[]), fx.group(&[]));
        let results: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = [&first, &second]
                .into_iter()
                .map(|group| scope.spawn(|| fx.groups.add(&fx.runs, &group.id, &run)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .any(|r| matches!(r, Err(GroupError::Conflict(_))))
        );
        let holders: Vec<_> = fx
            .stored()
            .into_iter()
            .filter(|group| group.runs.contains(&run))
            .collect();
        assert_eq!(holders.len(), 1);
    }
}

#[test]
fn a_member_whose_run_is_gone_is_shown_as_missing_and_can_be_detached() {
    let fx = fixture();
    let (kept, gone) = (fx.run(), fx.run());
    let group = fx.group(&[&kept, &gone]);
    // A hand edit of runs.json, or anything else that bypasses the group lock.
    assert!(fx.runs.delete(&gone).unwrap());
    let view = fx.groups.view(&fx.runs, &group.id).unwrap();
    assert!(!view.members[0].missing);
    assert!(view.members[1].missing);
    fx.groups.remove(&fx.runs, &group.id, &gone).unwrap();
    assert_eq!(fx.stored()[0].runs, [kept]);
}

#[test]
fn a_grouped_run_cannot_be_deleted_until_it_is_detached() {
    let fx = fixture();
    let run = fx.run();
    let group = fx.group(&[&run]);
    let refused = fx.groups.delete_run(&fx.runs, &run);
    assert!(matches!(refused, Err(GroupError::Conflict(m)) if m.contains(&group.id)));
    assert!(fx.runs.get(&run).unwrap().is_some());
    fx.groups.remove(&fx.runs, &group.id, &run).unwrap();
    assert!(fx.groups.delete_run(&fx.runs, &run).unwrap());
    assert!(!fx.groups.delete_run(&fx.runs, &run).unwrap());
}

#[test]
fn only_an_empty_group_can_be_deleted() {
    let fx = fixture();
    let run = fx.run();
    let group = fx.group(&[&run]);
    assert!(matches!(
        fx.groups.delete(&fx.runs, &group.id),
        Err(GroupError::Conflict(_))
    ));
    fx.groups.remove(&fx.runs, &group.id, &run).unwrap();
    fx.groups.delete(&fx.runs, &group.id).unwrap();
    assert!(fx.stored().is_empty());
    assert!(matches!(
        fx.groups.delete(&fx.runs, &group.id),
        Err(GroupError::NotFound(_))
    ));
}

#[test]
fn attachment_that_wins_the_race_makes_deletion_refuse() {
    let fx = fixture();
    let run = fx.run();
    let group = fx.group(&[]);
    fx.groups.add(&fx.runs, &group.id, &run).unwrap();
    assert!(matches!(
        fx.groups.delete_run(&fx.runs, &run),
        Err(GroupError::Conflict(_))
    ));
    assert!(fx.runs.get(&run).unwrap().is_some());
    assert_eq!(fx.stored()[0].runs, [run]);
}

#[test]
fn deletion_that_wins_the_race_makes_attachment_report_a_missing_run() {
    let fx = fixture();
    let run = fx.run();
    let group = fx.group(&[]);
    assert!(fx.groups.delete_run(&fx.runs, &run).unwrap());
    assert!(matches!(
        fx.groups.add(&fx.runs, &group.id, &run),
        Err(GroupError::NotFound(_))
    ));
    assert!(fx.runs.get(&run).unwrap().is_none());
    assert!(fx.stored()[0].runs.is_empty());
}

#[test]
fn attachment_racing_with_deletion_leaves_one_consistent_outcome() {
    for _ in 0..50 {
        let fx = fixture();
        let run = fx.run();
        let group = fx.group(&[]);
        let (added, deleted) = std::thread::scope(|scope| {
            let add = scope.spawn(|| fx.groups.add(&fx.runs, &group.id, &run));
            let delete = scope.spawn(|| fx.groups.delete_run(&fx.runs, &run));
            (add.join().unwrap(), delete.join().unwrap())
        });
        let exists = fx.runs.get(&run).unwrap().is_some();
        let member = fx.stored()[0].runs.contains(&run);
        match (added.is_ok(), deleted.is_ok()) {
            // Attachment won: the run is kept and deletion was refused.
            (true, false) => assert!(exists && member),
            // Deletion won: the run is gone and attachment reported it missing.
            (false, true) => assert!(!exists && !member),
            other => panic!("exactly one must win, got {other:?}"),
        }
    }
}

#[test]
fn a_summary_counts_attached_pull_requests_and_only_matching_statuses() {
    let fx = fixture();
    let (a, b, c) = (fx.run(), fx.run(), fx.run());
    let group = fx.group(&[&a, &b, &c]);
    let update = |id: &str, json: serde_json::Value| {
        fx.runs
            .update(id, serde_json::from_value(json).unwrap())
            .unwrap();
    };
    update(
        &a,
        serde_json::json!({"status": "running", "pr": "https://example.com/pr/1"}),
    );
    update(
        &b,
        serde_json::json!({"status": "running", "pr": "https://example.com/pr/2"}),
    );
    fx.runs
        .set_pr_status(
            &a,
            PrStatus {
                url: "https://example.com/pr/1".into(),
                state: crate::pr_state::PrState::Merged,
                reason: None,
                checked_at: 1,
            },
        )
        .unwrap();
    // A merged status of a replaced pull request does not count for the new one.
    fx.runs
        .set_pr_status(
            &b,
            PrStatus {
                url: "https://example.com/pr/old".into(),
                state: crate::pr_state::PrState::Merged,
                reason: None,
                checked_at: 1,
            },
        )
        .unwrap();
    let view = fx.groups.view(&fx.runs, &group.id).unwrap();
    assert_eq!(view.summary, "1 of 2 attached PRs merged");
    assert!(view.members[0].pr_status.is_some());
    assert!(view.members[1].pr_status.is_none());
    assert!(view.members[2].pr.is_none());
}
