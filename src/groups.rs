//! Change groups: ordinary runs linked to one shared spec. The dashboard server owns `groups.json` beside
//! `runs.json`; see ADR-0012.
//!
//! Lock order: every operation that needs both locks takes the group lock first, then the run lock. Checking
//! a run's membership and saving the change happen under one group lock, so a run cannot join two groups,
//! and cannot be deleted while another request attaches it.

use crate::{
    pr_state::PrStatus,
    run::{Run, RunStore, check_reference, check_text, lock_error, new_id, save_records},
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Longest group title; a title is one line in listings.
const MAX_TITLE_BYTES: usize = 200;
/// Longest spec reference, the same limit as a run's spec path.
const MAX_SPEC_BYTES: usize = 4096;
/// Runs one group can hold. A change touching more repositories than this is better split; see ADR-0012.
pub(crate) const MAX_MEMBERS: usize = 50;
/// Milestones and dependencies one group can hold, and the longest milestone text; see ADR-0013.
const MAX_MILESTONES: usize = 50;
const MAX_DEPENDENCIES: usize = 200;
const MAX_MILESTONE_BYTES: usize = 200;
/// Longest evidence URL, the same limit as a spec.
const MAX_EVIDENCE_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Group {
    pub(crate) id: String,
    pub(crate) title: String,
    /// A durable web link, or the absolute path of a local file.
    pub(crate) spec: String,
    /// Run ids, in the order they were attached.
    pub(crate) runs: Vec<String>,
    /// Conditions that happen outside the pull requests, created once and shared by the runs that wait on them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) milestones: Vec<Milestone>,
    /// What each member waits on. Advisory: nothing blocks on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) dependencies: Vec<Dependency>,
    /// Numbers milestone and dependency ids (`m1`, `d2`); an id is never reused.
    #[serde(default, skip_serializing_if = "is_zero")]
    next_id: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// A named condition, such as "Bicep module v2.4.0 published", confirmed by a person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Milestone {
    pub(crate) id: String,
    pub(crate) text: String,
    /// When a person confirmed it, in milliseconds since the Unix epoch; absent while pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) confirmed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) evidence: Option<String>,
}

/// What a dependency waits for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Target {
    /// The current pull request of this member run, merged.
    Pr(String),
    /// This milestone of the group, confirmed.
    Milestone(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Dependency {
    pub(crate) id: String,
    /// The member run that waits.
    pub(crate) run: String,
    pub(crate) target: Target,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewGroup {
    pub(crate) title: String,
    pub(crate) spec: String,
    #[serde(default)]
    pub(crate) runs: Vec<String>,
}

/// Why a group operation failed; the server maps each kind to a status.
#[derive(Debug)]
pub(crate) enum GroupError {
    Invalid(String),
    NotFound(String),
    Conflict(String),
    Storage(io::Error),
}

impl From<io::Error> for GroupError {
    fn from(error: io::Error) -> Self {
        Self::Storage(error)
    }
}

/// A group id is `g` and the digits of a run id's generator.
pub(crate) fn valid_group_id(id: &str) -> bool {
    id.strip_prefix('g')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

pub(crate) struct GroupStore {
    path: PathBuf,
    /// The group lock.
    groups: Mutex<Vec<Group>>,
}

impl GroupStore {
    /// Opens `groups.json` next to the given `runs.json`; a missing file is no groups.
    pub(crate) fn open(runs_path: &Path) -> io::Result<Self> {
        let path = runs_path.with_file_name("groups.json");
        let groups = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            path,
            groups: Mutex::new(groups),
        })
    }

    pub(crate) fn list(&self) -> Result<Vec<Group>, GroupError> {
        Ok(self.groups.lock().map_err(lock_error)?.clone())
    }

    /// Applies `edit` to a copy of the groups under the group lock and stores the copy when it succeeds, so a
    /// failed edit or save leaves the stored groups as they were.
    fn change<T>(
        &self,
        runs: &RunStore,
        edit: impl FnOnce(&mut Vec<Group>, &RunStore) -> Result<T, GroupError>,
    ) -> Result<T, GroupError> {
        let mut groups = self.groups.lock().map_err(lock_error)?;
        let mut next = groups.clone();
        let result = edit(&mut next, runs)?;
        save_records(&self.path, &next)?;
        *groups = next;
        Ok(result)
    }

    pub(crate) fn create(&self, runs: &RunStore, new: NewGroup) -> Result<Group, GroupError> {
        check_text("Title", &new.title, MAX_TITLE_BYTES).map_err(GroupError::Invalid)?;
        check_reference("Spec", &new.spec, MAX_SPEC_BYTES).map_err(GroupError::Invalid)?;
        let lower = new.spec.to_ascii_lowercase();
        let is_url = lower.starts_with("http://") || lower.starts_with("https://");
        if !is_url && !Path::new(&new.spec).is_absolute() {
            return Err(GroupError::Invalid(
                "Spec must be an HTTP(S) URL or an absolute path".to_string(),
            ));
        }
        if new.runs.len() > MAX_MEMBERS {
            return Err(GroupError::Invalid(format!(
                "A group holds at most {MAX_MEMBERS} runs"
            )));
        }
        self.change(runs, |groups, runs| {
            for (index, id) in new.runs.iter().enumerate() {
                if new.runs[..index].contains(id) {
                    return Err(GroupError::Invalid(format!("Run {id} is given twice")));
                }
                check_attachable(groups, runs, id)?;
            }
            let members = new.runs;
            let id = new_id(|candidate| {
                groups
                    .iter()
                    .any(|group| group.id == format!("g{candidate}"))
            })?;
            let group = Group {
                id: format!("g{id}"),
                title: new.title,
                spec: new.spec,
                runs: members,
                milestones: Vec::new(),
                dependencies: Vec::new(),
                next_id: 0,
            };
            groups.push(group.clone());
            Ok(group)
        })
    }

    pub(crate) fn add(&self, runs: &RunStore, group: &str, run: &str) -> Result<Group, GroupError> {
        self.change(runs, |groups, runs| {
            check_attachable(groups, runs, run)?;
            let found = find_mut(groups, group)?;
            if found.runs.len() >= MAX_MEMBERS {
                return Err(GroupError::Conflict(format!(
                    "Group {group} already holds {MAX_MEMBERS} runs"
                )));
            }
            found.runs.push(run.to_string());
            Ok(found.clone())
        })
    }

    /// Detaches `run`, whether or not its record still exists.
    pub(crate) fn remove(
        &self,
        runs: &RunStore,
        group: &str,
        run: &str,
    ) -> Result<Group, GroupError> {
        self.change(runs, |groups, _| {
            let found = find_mut(groups, group)?;
            let Some(index) = found.runs.iter().position(|member| member == run) else {
                return Err(GroupError::NotFound(format!(
                    "Run {run} is not in group {group}"
                )));
            };
            let blocking: Vec<&str> = found
                .dependencies
                .iter()
                .filter(|dependency| {
                    dependency.run == run || dependency.target == Target::Pr(run.to_string())
                })
                .map(|dependency| dependency.id.as_str())
                .collect();
            if !blocking.is_empty() {
                return Err(GroupError::Conflict(format!(
                    "Run {run} still has dependencies: {}. Remove them with `jidoka group undepend {group} <dependency-id>` first",
                    blocking.join(", ")
                )));
            }
            found.runs.remove(index);
            Ok(found.clone())
        })
    }

    /// Creates a pending milestone and returns its id.
    pub(crate) fn add_milestone(
        &self,
        runs: &RunStore,
        group: &str,
        text: &str,
    ) -> Result<String, GroupError> {
        check_text("Milestone", text, MAX_MILESTONE_BYTES).map_err(GroupError::Invalid)?;
        self.change(runs, |groups, _| {
            let found = find_mut(groups, group)?;
            if found.milestones.len() >= MAX_MILESTONES {
                return Err(GroupError::Conflict(format!(
                    "Group {group} already holds {MAX_MILESTONES} milestones"
                )));
            }
            let id = found.next_key('m');
            found.milestones.push(Milestone {
                id: id.clone(),
                text: text.to_string(),
                confirmed_at: None,
                evidence: None,
            });
            Ok(id)
        })
    }

    /// Deletes a milestone no dependency refers to.
    pub(crate) fn remove_milestone(
        &self,
        runs: &RunStore,
        group: &str,
        milestone: &str,
    ) -> Result<(), GroupError> {
        self.change(runs, |groups, _| {
            let found = find_mut(groups, group)?;
            let index = milestone_index(found, milestone)?;
            let users: Vec<&str> = found
                .dependencies
                .iter()
                .filter(|dependency| dependency.target == Target::Milestone(milestone.to_string()))
                .map(|dependency| dependency.id.as_str())
                .collect();
            if !users.is_empty() {
                return Err(GroupError::Conflict(format!(
                    "Milestone {milestone} is still needed by dependencies: {}. Remove them with `jidoka group undepend {group} <dependency-id>` first",
                    users.join(", ")
                )));
            }
            found.milestones.remove(index);
            Ok(())
        })
    }

    /// Makes member `run` wait on `target` and returns the dependency id. Both ends must be in the group, a
    /// run cannot wait on its own pull request, and the pull request dependencies cannot form a cycle.
    pub(crate) fn add_dependency(
        &self,
        runs: &RunStore,
        group: &str,
        run: &str,
        target: Target,
    ) -> Result<String, GroupError> {
        self.change(runs, |groups, _| {
            let found = find_mut(groups, group)?;
            check_member(found, run)?;
            match &target {
                Target::Pr(upstream) => {
                    check_member(found, upstream)?;
                    if upstream == run {
                        return Err(GroupError::Invalid(format!(
                            "Run {run} cannot wait on its own pull request"
                        )));
                    }
                    if waits_on(found, upstream, run) {
                        return Err(GroupError::Invalid(format!(
                            "Run {upstream} already waits on run {run}'s pull request, directly or through other runs; that would be a cycle"
                        )));
                    }
                }
                Target::Milestone(milestone) => {
                    milestone_index(found, milestone)?;
                }
            }
            if found
                .dependencies
                .iter()
                .any(|dependency| dependency.run == run && dependency.target == target)
            {
                return Err(GroupError::Conflict(format!(
                    "Run {run} already has that dependency"
                )));
            }
            if found.dependencies.len() >= MAX_DEPENDENCIES {
                return Err(GroupError::Conflict(format!(
                    "Group {group} already holds {MAX_DEPENDENCIES} dependencies"
                )));
            }
            let id = found.next_key('d');
            found.dependencies.push(Dependency {
                id: id.clone(),
                run: run.to_string(),
                target,
            });
            Ok(id)
        })
    }

    /// Removes one run's dependency; other runs waiting on the same target keep theirs.
    pub(crate) fn remove_dependency(
        &self,
        runs: &RunStore,
        group: &str,
        dependency: &str,
    ) -> Result<(), GroupError> {
        self.change(runs, |groups, _| {
            let found = find_mut(groups, group)?;
            let index = found
                .dependencies
                .iter()
                .position(|candidate| candidate.id == dependency)
                .ok_or_else(|| {
                    GroupError::NotFound(format!("No dependency {dependency} in group {group}"))
                })?;
            found.dependencies.remove(index);
            Ok(())
        })
    }

    /// Records one confirmation, at `now` (milliseconds since the Unix epoch), for every run that waits on the
    /// milestone. Who confirmed is not recorded: Jidoka has no user identity.
    pub(crate) fn confirm(
        &self,
        runs: &RunStore,
        group: &str,
        milestone: &str,
        evidence: Option<&str>,
        now: u64,
    ) -> Result<(), GroupError> {
        if let Some(evidence) = evidence {
            check_reference("Evidence", evidence, MAX_EVIDENCE_BYTES)
                .map_err(GroupError::Invalid)?;
            let lower = evidence.to_ascii_lowercase();
            if !lower.starts_with("http://") && !lower.starts_with("https://") {
                return Err(GroupError::Invalid(
                    "Evidence must be an HTTP(S) URL".to_string(),
                ));
            }
        }
        self.change(runs, |groups, _| {
            let found = find_mut(groups, group)?;
            let index = milestone_index(found, milestone)?;
            let target = &mut found.milestones[index];
            if target.confirmed_at.is_some() {
                return Err(GroupError::Conflict(format!(
                    "Milestone {milestone} is already confirmed; reopen it with `jidoka group reopen {group} {milestone}` to confirm again"
                )));
            }
            target.confirmed_at = Some(now);
            target.evidence = evidence.map(str::to_string);
            Ok(())
        })
    }

    /// Returns a confirmed milestone to pending and clears its time and evidence.
    pub(crate) fn reopen(
        &self,
        runs: &RunStore,
        group: &str,
        milestone: &str,
    ) -> Result<(), GroupError> {
        self.change(runs, |groups, _| {
            let found = find_mut(groups, group)?;
            let index = milestone_index(found, milestone)?;
            let target = &mut found.milestones[index];
            if target.confirmed_at.is_none() {
                return Err(GroupError::Conflict(format!(
                    "Milestone {milestone} is not confirmed"
                )));
            }
            target.confirmed_at = None;
            target.evidence = None;
            Ok(())
        })
    }

    /// Deletes a group that has no members.
    pub(crate) fn delete(&self, runs: &RunStore, group: &str) -> Result<(), GroupError> {
        self.change(runs, |groups, _| {
            let index = groups
                .iter()
                .position(|candidate| candidate.id == group)
                .ok_or_else(|| GroupError::NotFound(format!("No group {group}")))?;
            if !groups[index].runs.is_empty() {
                return Err(GroupError::Conflict(format!(
                    "Group {group} still has runs: {}. Detach them with `jidoka group remove {group} <run-id>` first",
                    groups[index].runs.join(", ")
                )));
            }
            groups.remove(index);
            Ok(())
        })
    }

    /// Deletes a run record unless a group holds it. The group lock is held from the membership check to the
    /// deletion, so an attachment cannot slip in between.
    pub(crate) fn delete_run(&self, runs: &RunStore, id: &str) -> Result<bool, GroupError> {
        let groups = self.groups.lock().map_err(lock_error)?;
        if let Some(group) = groups
            .iter()
            .find(|group| group.runs.iter().any(|member| member == id))
        {
            return Err(GroupError::Conflict(format!(
                "Run {id} belongs to group {0}; detach it first with `jidoka group remove {0} {id}`",
                group.id
            )));
        }
        Ok(runs.delete(id)?)
    }

    /// One group with its members' repository and pull request, as stored on the runs now.
    pub(crate) fn view(&self, runs: &RunStore, group: &str) -> Result<GroupView, GroupError> {
        let groups = self.groups.lock().map_err(lock_error)?;
        let found = groups
            .iter()
            .find(|candidate| candidate.id == group)
            .ok_or_else(|| GroupError::NotFound(format!("No group {group}")))?;
        let all = runs.list()?;
        Ok(GroupView::new(found, &all))
    }
}

fn find_mut<'a>(groups: &'a mut [Group], id: &str) -> Result<&'a mut Group, GroupError> {
    groups
        .iter_mut()
        .find(|group| group.id == id)
        .ok_or_else(|| GroupError::NotFound(format!("No group {id}")))
}

impl Group {
    /// The next unused id, such as `m1` or `d2`.
    fn next_key(&mut self, prefix: char) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }
}

fn check_member(group: &Group, run: &str) -> Result<(), GroupError> {
    if group.runs.iter().any(|member| member == run) {
        Ok(())
    } else {
        Err(GroupError::NotFound(format!(
            "Run {run} is not in group {}",
            group.id
        )))
    }
}

fn milestone_index(group: &Group, milestone: &str) -> Result<usize, GroupError> {
    group
        .milestones
        .iter()
        .position(|candidate| candidate.id == milestone)
        .ok_or_else(|| {
            GroupError::NotFound(format!("No milestone {milestone} in group {}", group.id))
        })
}

/// Whether `from` waits on the pull request of `to`, directly or through other runs.
fn waits_on(group: &Group, from: &str, to: &str) -> bool {
    let mut seen = vec![from];
    let mut index = 0;
    while let Some(&current) = seen.get(index) {
        index += 1;
        for dependency in &group.dependencies {
            if dependency.run != current {
                continue;
            }
            if let Target::Pr(next) = &dependency.target {
                if next == to {
                    return true;
                }
                if !seen.contains(&next.as_str()) {
                    seen.push(next);
                }
            }
        }
    }
    false
}

/// The run exists and is in no group.
fn check_attachable(groups: &[Group], runs: &RunStore, run: &str) -> Result<(), GroupError> {
    if let Some(group) = groups
        .iter()
        .find(|group| group.runs.iter().any(|member| member == run))
    {
        return Err(GroupError::Conflict(format!(
            "Run {run} is already in group {}",
            group.id
        )));
    }
    if runs.get(run)?.is_none() {
        return Err(GroupError::NotFound(format!("No run {run}")));
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GroupView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) spec: String,
    pub(crate) members: Vec<Member>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) milestones: Vec<MilestoneView>,
    pub(crate) summary: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Member {
    pub(crate) run: String,
    /// The group names a run whose record is gone (for example after a hand edit of `runs.json`).
    pub(crate) missing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pr: Option<String>,
    /// The stored state of `pr`; absent while the stored state belongs to another pull request or none was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pr_status: Option<PrStatus>,
    /// What this run waits on, with whether each condition holds now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) dependencies: Vec<DependencyView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DependencyView {
    pub(crate) id: String,
    pub(crate) target: Target,
    /// What is waited for, in words: the upstream pull request, or the milestone text.
    pub(crate) description: String,
    /// Whether the condition holds now.
    pub(crate) cleared: bool,
    /// Where the condition stands, such as `merged`, `open`, `unchecked` or `pending`.
    pub(crate) status: String,
    /// When the upstream pull request's state was read, only while it belongs to its current URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checked_at: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MilestoneView {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) confirmed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) evidence: Option<String>,
    /// The runs that wait on it; empty when nothing does.
    pub(crate) runs: Vec<String>,
}

/// Where the pull request of `run` stands as a prerequisite. Only a state read for the run's current URL
/// counts, so a previous pull request's merge never clears its replacement.
fn pr_condition(run: Option<&Run>) -> (bool, String, Option<u64>) {
    let Some(run) = run else {
        return (false, "run missing".to_string(), None);
    };
    let Some(url) = &run.pr else {
        return (false, "no pull request recorded".to_string(), None);
    };
    let Some(status) = run.pr_status.as_ref().filter(|status| &status.url == url) else {
        return (false, "unchecked".to_string(), None);
    };
    let state = match status.state {
        crate::pr_state::PrState::Merged => "merged".to_string(),
        crate::pr_state::PrState::Open => "open".to_string(),
        crate::pr_state::PrState::Draft => "a draft".to_string(),
        crate::pr_state::PrState::Closed => "closed without merge".to_string(),
        crate::pr_state::PrState::Unknown => "unknown".to_string(),
    };
    (
        status.state == crate::pr_state::PrState::Merged,
        state,
        Some(status.checked_at),
    )
}

/// What `dependency` waits for and whether that holds now.
fn dependency_view(group: &Group, runs: &[Run], dependency: &Dependency) -> DependencyView {
    let (description, cleared, status, checked_at) = match &dependency.target {
        Target::Pr(upstream) => {
            let run = runs.iter().find(|run| &run.id == upstream);
            let (cleared, status, checked_at) = pr_condition(run);
            let place = run
                .and_then(|run| run.repository.as_deref())
                .map_or(String::new(), |repository| format!(" ({repository})"));
            (
                format!("pull request of run {upstream}{place}"),
                cleared,
                status,
                checked_at,
            )
        }
        Target::Milestone(id) => {
            let milestone = group.milestones.iter().find(|m| &m.id == id);
            let cleared = milestone.is_some_and(|m| m.confirmed_at.is_some());
            (
                milestone.map_or_else(
                    || format!("milestone {id}"),
                    |m| format!("{id}: {}", m.text),
                ),
                cleared,
                if cleared { "confirmed" } else { "pending" }.to_string(),
                None,
            )
        }
    };
    DependencyView {
        id: dependency.id.clone(),
        target: dependency.target.clone(),
        description,
        cleared,
        status,
        checked_at,
    }
}

/// One member: the run's repository and pull request as stored now, and what it waits on.
fn member_view(group: &Group, runs: &[Run], id: &str) -> Member {
    let dependencies = group
        .dependencies
        .iter()
        .filter(|dependency| dependency.run == id)
        .map(|dependency| dependency_view(group, runs, dependency))
        .collect();
    match runs.iter().find(|run| run.id == id) {
        Some(run) => Member {
            run: id.to_string(),
            missing: false,
            title: Some(run.title.clone()),
            repository: run.repository.clone(),
            pr: run.pr.clone(),
            pr_status: run
                .pr_status
                .clone()
                .filter(|status| run.pr.as_deref() == Some(status.url.as_str())),
            dependencies,
        },
        None => Member {
            run: id.to_string(),
            missing: true,
            title: None,
            repository: None,
            pr: None,
            pr_status: None,
            dependencies,
        },
    }
}

fn milestone_view(group: &Group, milestone: &Milestone) -> MilestoneView {
    MilestoneView {
        id: milestone.id.clone(),
        text: milestone.text.clone(),
        confirmed: milestone.confirmed_at.is_some(),
        confirmed_at: milestone.confirmed_at,
        evidence: milestone.evidence.clone(),
        runs: group
            .dependencies
            .iter()
            .filter(|dependency| dependency.target == Target::Milestone(milestone.id.clone()))
            .map(|dependency| dependency.run.clone())
            .collect(),
    }
}

impl GroupView {
    fn new(group: &Group, runs: &[Run]) -> Self {
        let members: Vec<Member> = group
            .runs
            .iter()
            .map(|id| member_view(group, runs, id))
            .collect();
        let attached = members.iter().filter(|member| member.pr.is_some()).count();
        let merged = members
            .iter()
            .filter(|member| {
                member
                    .pr_status
                    .as_ref()
                    .is_some_and(|status| status.state == crate::pr_state::PrState::Merged)
            })
            .count();
        let milestones: Vec<MilestoneView> = group
            .milestones
            .iter()
            .map(|milestone| milestone_view(group, milestone))
            .collect();
        let mut summary = format!("{merged} of {attached} attached PRs merged");
        if !milestones.is_empty() {
            // Each pending milestone counts once, however many runs wait on it.
            let pending = milestones.iter().filter(|m| !m.confirmed).count();
            let noun = if pending == 1 {
                "milestone"
            } else {
                "milestones"
            };
            summary.push_str(&format!(", {pending} {noun} pending"));
        }
        Self {
            id: group.id.clone(),
            title: group.title.clone(),
            spec: group.spec.clone(),
            summary,
            members,
            milestones,
        }
    }
}

#[cfg(test)]
mod tests;
