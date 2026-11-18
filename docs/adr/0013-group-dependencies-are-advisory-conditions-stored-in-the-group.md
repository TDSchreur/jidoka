---
status: accepted
---

# Group dependencies are advisory conditions stored in the group

[ADR-0012](0012-change-groups-link-runs-through-a-server-owned-groups-file.md) decided that groups are advisory and that a pull request's merge must match its current URL. This record fixes the format and the rules that make those decisions work, so step 2 of change groups (milestones, dependencies and Finish guidance) has one contract.

- **Where they live.** A group record gains `milestones`, `dependencies` and a counter, all optional, so a group stored by step 1 loads unchanged. Dependencies are not a field of the run, for the reason ADR-0012 rejected a `group` field on runs: validation spans several runs and must happen under the group lock.
- **IDs.** A milestone is `m<n>` and a dependency is `d<n>`, numbered from one counter per group that never goes back, so an ID that was deleted is never reused for something else.
- **Two kinds of condition.** A dependency makes one member run wait on either the current pull request of another member (cleared when that run's stored status matches its current URL and is merged) or a milestone of the same group (cleared while a person has confirmed it). A merge never clears a milestone.
- **Rules.** Both ends must be members of the group. A run cannot wait on itself, and pull request dependencies cannot form a cycle. The same dependency cannot be added twice. A group holds at most 50 milestones and 200 dependencies; a milestone's text is at most 200 bytes.
- **Detaching and deleting.** `group remove` refuses a run that waits on something or is waited on, and names the dependencies; `milestone remove` refuses a milestone that is still referenced. Nothing is removed implicitly, so no one loses a prerequisite without noticing.
- **Confirming.** `confirm` stores the time and an optional `http(s)` evidence URL; it refuses a milestone that is already confirmed, so evidence is never overwritten silently, and `reopen` clears both. Who confirmed is not stored, because Jidoka has no user identity.
- **Reading, not enforcing.** Group inspection, `jidoka run next` in Finish and later the dashboard show the conditions; nothing blocks on them. A condition whose pull request has no matching stored status reads as `unchecked` and stays pending. The summary counts each pending milestone once, however many runs wait on it.
- **Local specs.** `jidoka run next` leaves a local spec file's path out, so an agent cannot copy it into a pull request. The agent describes the prerequisites instead.

## Considered Options

- **Enforce dependencies (block Finish or merge).** Rejected: a platform change often spans work outside Jidoka, so a block would be wrong as often as right, and ADR-0012 already chose advisory.
- **A milestone per dependent run.** Rejected: one publication would need one confirmation per consumer, and they could disagree.
- **Overwrite on a second `confirm`.** Rejected: it replaces the evidence a reviewer may be relying on without saying so. `reopen` first makes the replacement deliberate.
- **Implicit cleanup when detaching a run.** Rejected: it would hide that a prerequisite disappeared.

## Consequences

- Restart the dashboard after upgrading so it serves the new `/api/groups/<id>/...` routes; `jidoka run next` in Finish reads them for a grouped run.
- The group view and `group inspect --json` gain `milestones` and per-member `dependencies`; ungrouped runs and groups without milestones keep their output.
- No migration: a group without milestones or dependencies behaves as before.
