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

const MAX_TITLE_BYTES: usize = 200;
const MAX_SPEC_BYTES: usize = 4096;
/// Runs one group can hold, so a request body stays small.
pub(crate) const MAX_MEMBERS: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Group {
    pub(crate) id: String,
    pub(crate) title: String,
    /// A durable web link, or the absolute path of a local file.
    pub(crate) spec: String,
    /// Run ids, in the order they were attached.
    pub(crate) runs: Vec<String>,
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
        if new.runs.len() > MAX_MEMBERS {
            return Err(GroupError::Invalid(format!(
                "A group holds at most {MAX_MEMBERS} runs"
            )));
        }
        self.change(runs, |groups, runs| {
            let mut members: Vec<String> = Vec::new();
            for id in new.runs {
                if !members.contains(&id) {
                    check_attachable(groups, runs, &id)?;
                    members.push(id);
                }
            }
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
            found.runs.remove(index);
            Ok(found.clone())
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
}

impl GroupView {
    fn new(group: &Group, runs: &[Run]) -> Self {
        let members: Vec<Member> = group
            .runs
            .iter()
            .map(|id| match runs.iter().find(|run| &run.id == id) {
                Some(run) => Member {
                    run: id.clone(),
                    missing: false,
                    title: Some(run.title.clone()),
                    repository: run.repository.clone(),
                    pr: run.pr.clone(),
                    pr_status: run
                        .pr_status
                        .clone()
                        .filter(|status| run.pr.as_deref() == Some(status.url.as_str())),
                },
                None => Member {
                    run: id.clone(),
                    missing: true,
                    title: None,
                    repository: None,
                    pr: None,
                    pr_status: None,
                },
            })
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
        Self {
            id: group.id.clone(),
            title: group.title.clone(),
            spec: group.spec.clone(),
            summary: format!("{merged} of {attached} attached PRs merged"),
            members,
        }
    }
}

#[cfg(test)]
mod tests;
