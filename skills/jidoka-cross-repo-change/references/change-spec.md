# Change spec (fallback template)

Use when the knowledge repo has no `changes/_template.md`.

```markdown
# <Change title>

Status: draft | in progress | done
Group: <Jidoka group id, once created>

## Goal

<What changes and why.>

## Repositories and pull requests

| Repository | Pull request | State | Waits on |
| --- | --- | --- | --- |
| <name> | <link> | open, merged | <pull request or milestone> |

## Milestones

| Milestone | Status | Evidence |
| --- | --- | --- |
| <what must be true> | pending, confirmed | <link> |

## Decisions and open questions

- <decision, with the reason>

## Rollout notes

<Order of deployment, rollback, who to tell.>
```
