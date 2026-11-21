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
    let group = fx.group(&[&a, &b]);
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
    for (title, spec) in [
        (" ", "https://example.com"),
        ("t", ""),
        ("t", "ftp://x"),
        ("t", "relative/spec.md"),
    ] {
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
    update(&a, serde_json::json!({"pr": "https://example.com/pr/1"}));
    update(&b, serde_json::json!({"pr": "https://example.com/pr/2"}));
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

#[test]
fn a_run_given_twice_is_refused_and_nothing_is_saved() {
    let fx = fixture();
    let run = fx.run();
    let result = fx.groups.create(
        &fx.runs,
        NewGroup {
            title: "t".into(),
            spec: "https://example.com".into(),
            runs: vec![run.clone(), run],
        },
    );
    assert!(matches!(result, Err(GroupError::Invalid(_))));
    assert!(fx.stored().is_empty());
}

fn merged(url: &str) -> PrStatus {
    PrStatus {
        url: url.into(),
        state: crate::pr_state::PrState::Merged,
        reason: None,
        checked_at: 5,
    }
}

impl Fixture {
    fn open_pr(&self, run: &str, url: &str) {
        self.runs
            .update(
                run,
                serde_json::from_value(serde_json::json!({"pr": url})).unwrap(),
            )
            .unwrap();
    }

    fn view(&self, group: &str) -> GroupView {
        self.groups.view(&self.runs, group).unwrap()
    }
}

#[test]
fn invalid_dependency_edges_are_rejected() {
    let fx = fixture();
    let (a, b, c, outsider) = (fx.run(), fx.run(), fx.run(), fx.run());
    let group = fx.group(&[&a, &b, &c]);
    let other = fx.group(&[&outsider]);
    let add =
        |run: &str, target: Target| fx.groups.add_dependency(&fx.runs, &group.id, run, target);
    // Itself, a run outside the group, and a milestone that does not exist.
    assert!(matches!(
        add(&a, Target::Pr(a.clone())),
        Err(GroupError::Invalid(_))
    ));
    assert!(matches!(
        add(&a, Target::Pr(outsider.clone())),
        Err(GroupError::NotFound(_))
    ));
    assert!(matches!(
        add(&outsider, Target::Pr(a.clone())),
        Err(GroupError::NotFound(_))
    ));
    assert!(matches!(
        add(&a, Target::Milestone("m9".into())),
        Err(GroupError::NotFound(_))
    ));
    // A milestone of another group is unknown in this one.
    let foreign = fx
        .groups
        .add_milestone(&fx.runs, &other.id, "elsewhere")
        .unwrap();
    assert!(matches!(
        add(&a, Target::Milestone(foreign)),
        Err(GroupError::NotFound(_))
    ));
    // A direct cycle, a longer one, and a repeated dependency.
    add(&a, Target::Pr(b.clone())).unwrap();
    assert!(matches!(
        add(&b, Target::Pr(a.clone())),
        Err(GroupError::Invalid(_))
    ));
    add(&b, Target::Pr(c.clone())).unwrap();
    assert!(matches!(
        add(&c, Target::Pr(a.clone())),
        Err(GroupError::Invalid(_))
    ));
    assert!(matches!(
        add(&a, Target::Pr(b)),
        Err(GroupError::Conflict(_))
    ));
    assert_eq!(fx.stored()[0].dependencies.len(), 2);
}

#[test]
fn a_run_with_dependencies_cannot_be_detached_and_a_needed_milestone_cannot_be_removed() {
    let fx = fixture();
    let (module, api) = (fx.run(), fx.run());
    let group = fx.group(&[&module, &api]);
    let milestone = fx
        .groups
        .add_milestone(&fx.runs, &group.id, "published")
        .unwrap();
    let on_pr = fx
        .groups
        .add_dependency(&fx.runs, &group.id, &api, Target::Pr(module.clone()))
        .unwrap();
    let on_milestone = fx
        .groups
        .add_dependency(
            &fx.runs,
            &group.id,
            &api,
            Target::Milestone(milestone.clone()),
        )
        .unwrap();
    // Both the waiting run and the awaited run are held by the dependency.
    for run in [&module, &api] {
        let refused = fx.groups.remove(&fx.runs, &group.id, run);
        assert!(
            matches!(refused, Err(GroupError::Conflict(m)) if m.contains(&on_pr)),
            "{run}"
        );
    }
    let refused = fx.groups.remove_milestone(&fx.runs, &group.id, &milestone);
    assert!(matches!(refused, Err(GroupError::Conflict(m)) if m.contains(&on_milestone)));
    fx.groups
        .remove_dependency(&fx.runs, &group.id, &on_milestone)
        .unwrap();
    fx.groups
        .remove_milestone(&fx.runs, &group.id, &milestone)
        .unwrap();
    fx.groups
        .remove_dependency(&fx.runs, &group.id, &on_pr)
        .unwrap();
    fx.groups.remove(&fx.runs, &group.id, &api).unwrap();
    assert!(matches!(
        fx.groups.remove_dependency(&fx.runs, &group.id, &on_pr),
        Err(GroupError::NotFound(_))
    ));
}

#[test]
fn ids_are_never_reused() {
    let fx = fixture();
    let group = fx.group(&[]);
    let first = fx.groups.add_milestone(&fx.runs, &group.id, "one").unwrap();
    fx.groups
        .remove_milestone(&fx.runs, &group.id, &first)
        .unwrap();
    let second = fx.groups.add_milestone(&fx.runs, &group.id, "two").unwrap();
    assert_ne!(first, second);
    assert!(fx.groups.add_milestone(&fx.runs, &group.id, " ").is_err());
}

#[test]
fn confirming_and_reopening_a_milestone_applies_to_both_consumers_and_counts_it_once() {
    let fx = fixture();
    let (infra, api) = (fx.run(), fx.run());
    let group = fx.group(&[&infra, &api]);
    let milestone = fx
        .groups
        .add_milestone(&fx.runs, &group.id, "v2.4.0 published")
        .unwrap();
    for run in [&infra, &api] {
        fx.groups
            .add_dependency(
                &fx.runs,
                &group.id,
                run,
                Target::Milestone(milestone.clone()),
            )
            .unwrap();
    }
    let unused = fx
        .groups
        .add_milestone(&fx.runs, &group.id, "nobody waits")
        .unwrap();
    let view = fx.view(&group.id);
    assert_eq!(
        view.summary,
        "0 of 0 attached PRs merged, 2 milestones pending"
    );
    assert_eq!(view.milestones[0].runs, [infra.clone(), api.clone()]);
    assert!(
        view.milestones
            .iter()
            .any(|m| m.id == unused && m.runs.is_empty())
    );
    assert!(view.members.iter().all(|m| !m.dependencies[0].cleared));

    let evidence = "https://example.com/feed/v2.4.0";
    fx.groups
        .confirm(&fx.runs, &group.id, &milestone, Some(evidence), 42)
        .unwrap();
    let view = fx.view(&group.id);
    assert!(view.members.iter().all(|m| m.dependencies[0].cleared));
    assert_eq!(view.milestones[0].confirmed_at, Some(42));
    assert_eq!(view.milestones[0].evidence.as_deref(), Some(evidence));
    assert!(
        view.summary.ends_with("1 milestone pending"),
        "{}",
        view.summary
    );
    // A second confirmation would overwrite the evidence silently.
    assert!(matches!(
        fx.groups.confirm(&fx.runs, &group.id, &milestone, None, 43),
        Err(GroupError::Conflict(_))
    ));

    fx.groups.reopen(&fx.runs, &group.id, &milestone).unwrap();
    let view = fx.view(&group.id);
    assert!(view.members.iter().all(|m| !m.dependencies[0].cleared));
    assert_eq!(view.milestones[0].confirmed_at, None);
    assert_eq!(view.milestones[0].evidence, None);
    assert!(matches!(
        fx.groups.reopen(&fx.runs, &group.id, &milestone),
        Err(GroupError::Conflict(_))
    ));
    for bad in ["ftp://x", "not a url", "/local/file"] {
        let result = fx
            .groups
            .confirm(&fx.runs, &group.id, &milestone, Some(bad), 1);
        assert!(matches!(result, Err(GroupError::Invalid(_))), "{bad}");
    }
    // Nothing above stored a confirmation.
    assert_eq!(fx.stored()[0].milestones[0].confirmed_at, None);
}

#[test]
fn a_pull_request_dependency_clears_only_for_the_current_urls_merged_status() {
    let fx = fixture();
    let (module, infra) = (fx.run(), fx.run());
    let group = fx.group(&[&module, &infra]);
    fx.groups
        .add_dependency(&fx.runs, &group.id, &infra, Target::Pr(module.clone()))
        .unwrap();
    let state = |fx: &Fixture| {
        let dependency = &fx.view(&group.id).members[1].dependencies[0];
        (
            dependency.cleared,
            dependency.status.clone(),
            dependency.checked_at,
        )
    };
    assert_eq!(
        state(&fx),
        (false, "no pull request recorded".to_string(), None)
    );
    fx.open_pr(&module, "https://example.com/pr/1");
    assert_eq!(state(&fx), (false, "unchecked".to_string(), None));
    let set = |state, url: &str| {
        fx.runs
            .set_pr_status(
                &module,
                PrStatus {
                    url: url.into(),
                    state,
                    reason: None,
                    checked_at: 9,
                },
            )
            .unwrap();
    };
    use crate::pr_state::PrState;
    for (pr_state, shown) in [
        (PrState::Open, "open"),
        (PrState::Draft, "a draft"),
        (PrState::Closed, "closed without merge"),
        (PrState::Unknown, "unknown"),
    ] {
        set(pr_state, "https://example.com/pr/1");
        assert_eq!(state(&fx), (false, shown.to_string(), Some(9)));
    }
    set(PrState::Merged, "https://example.com/pr/1");
    assert_eq!(state(&fx), (true, "merged".to_string(), Some(9)));
    // The run gets a replacement pull request that nobody has checked yet: the old merge does not count.
    fx.open_pr(&module, "https://example.com/pr/2");
    assert_eq!(state(&fx), (false, "unchecked".to_string(), None));
    set(PrState::Merged, "https://example.com/pr/2");
    assert!(state(&fx).0);
    // A missing upstream run record keeps the dependency pending.
    assert!(fx.runs.delete(&module).unwrap());
    assert_eq!(state(&fx), (false, "run missing".to_string(), None));
}

/// The plan's acceptance scenario, on the URL shapes of both hosts: only the module pull request merging clears
/// the pull request dependency, the milestone stays pending until confirmed, and a replacement pull request
/// has to be checked itself.
#[test]
fn the_acceptance_scenario_holds_for_github_and_azure_devops_pull_requests() {
    for (module_pr, replacement_pr, infra_pr, api_pr) in [
        (
            "https://github.com/acme/modules/pull/7",
            "https://github.com/acme/modules/pull/9",
            "https://github.com/acme/infra/pull/3",
            "https://github.com/acme/api/pull/12",
        ),
        (
            "https://dev.azure.com/acme/platform/_git/modules/pullrequest/7",
            "https://dev.azure.com/acme/platform/_git/modules/pullrequest/9",
            "https://dev.azure.com/acme/platform/_git/infra/pullrequest/3",
            "https://dev.azure.com/acme/platform/_git/api/pullrequest/12",
        ),
    ] {
        let fx = fixture();
        let (module, infra, api) = (fx.run(), fx.run(), fx.run());
        let group = fx.group(&[&module, &infra, &api]);
        let milestone = fx
            .groups
            .add_milestone(&fx.runs, &group.id, "Bicep module v2.4.0 published")
            .unwrap();
        for run in [&infra, &api] {
            fx.groups
                .add_dependency(
                    &fx.runs,
                    &group.id,
                    run,
                    Target::Milestone(milestone.clone()),
                )
                .unwrap();
        }
        fx.groups
            .add_dependency(&fx.runs, &group.id, &infra, Target::Pr(module.clone()))
            .unwrap();
        for (run, url) in [(&module, module_pr), (&infra, infra_pr), (&api, api_pr)] {
            fx.open_pr(run, url);
        }
        let outstanding = |fx: &Fixture, run: usize| {
            fx.view(&group.id).members[run]
                .dependencies
                .iter()
                .filter(|d| !d.cleared)
                .map(|d| d.id.clone())
                .collect::<Vec<_>>()
                .len()
        };
        // The module pull request merges: only the pull request dependency clears.
        fx.runs.set_pr_status(&module, merged(module_pr)).unwrap();
        let view = fx.view(&group.id);
        assert_eq!(
            view.summary,
            "1 of 3 attached PRs merged, 1 milestone pending"
        );
        assert_eq!(outstanding(&fx, 1), 1, "infra still waits on the milestone");
        assert_eq!(outstanding(&fx, 2), 1, "api still waits on the milestone");
        assert!(view.members[1].dependencies.iter().any(|d| d.cleared));
        // Publication is confirmed and reopened: both consumers follow.
        fx.groups
            .confirm(
                &fx.runs,
                &group.id,
                &milestone,
                Some("https://example.com/e"),
                1,
            )
            .unwrap();
        assert_eq!((outstanding(&fx, 1), outstanding(&fx, 2)), (0, 0));
        fx.groups.reopen(&fx.runs, &group.id, &milestone).unwrap();
        assert_eq!((outstanding(&fx, 1), outstanding(&fx, 2)), (1, 1));
        // The module run's pull request is replaced and not yet checked: its dependency is pending again.
        fx.groups
            .confirm(&fx.runs, &group.id, &milestone, None, 2)
            .unwrap();
        fx.open_pr(&module, replacement_pr);
        assert_eq!(outstanding(&fx, 1), 1);
        fx.runs
            .set_pr_status(&module, merged(replacement_pr))
            .unwrap();
        assert_eq!(outstanding(&fx, 1), 0);
        // A grouped run cannot be deleted, and detaching one lists what holds it.
        assert!(matches!(
            fx.groups.delete_run(&fx.runs, &module),
            Err(GroupError::Conflict(_))
        ));
        assert!(matches!(
            fx.groups.remove(&fx.runs, &group.id, &module),
            Err(GroupError::Conflict(_))
        ));
    }
}

#[test]
fn groups_stored_before_dependencies_still_load() {
    let dir = TestDir::new();
    let runs_path = dir.0.path().join("runs.json");
    std::fs::write(
        dir.0.path().join("groups.json"),
        r#"[{"id":"g1","title":"t","spec":"https://example.com","runs":["1"]}]"#,
    )
    .unwrap();
    let groups = GroupStore::open(&runs_path).unwrap().list().unwrap();
    assert!(groups[0].milestones.is_empty() && groups[0].dependencies.is_empty());
}
