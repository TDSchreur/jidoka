# Templates for a new knowledge repo

Replace placeholders in angle brackets. Keep notes short; a reader should find the answer in one screen.

## README.md

```markdown
# <Platform name> knowledge

Shared knowledge about <platform>, and the specs of changes that touch several repositories. People and coding agents read it before a cross-repo change and add what they learn.

## Layout

- `platform/` how the platform fits together: `overview.md`, `repos.md`, `environments.md`
- `changes/` one spec per cross-repo change, `changes/<name>.md`, from `_template.md`
- `learnings/` facts agents discovered, one note per topic; unverified ones are marked
- `recipes/` how to do common cross-repo changes

## For agents

- Read `platform/` before starting a cross-repo change.
- Pull with rebase, make one change, commit it alone, push to the default branch. On a rejected push, pull again and retry.
- Record only durable platform facts that are not already here, with date and source. Never record secrets, tokens, credentials, personal data or customer data.
- This repo lists no local checkout paths; those stay on each person's machine.
```

## platform/repos.md

```markdown
# Repositories

One row per repository. No local paths.

| Name | Host | Remote | Role | Owner | Default branch |
| --- | --- | --- | --- | --- | --- |
| <name> | GitHub or Azure DevOps | <remote URL> | <module, infrastructure, API, pipeline, ...> | <team or person> | <branch> |
```

## platform/overview.md

```markdown
# Platform overview

<Two or three sentences on what the platform is.>

## How the repositories relate

<Which repository consumes or deploys which. A short list or diagram.>

## Shipping order

<What must be merged, published or deployed before what.>
```

## platform/environments.md

```markdown
# Environments

| Environment | Purpose | Deployed from | Notes |
| --- | --- | --- | --- |
| <name> | <purpose> | <repository and branch or pipeline> | <conventions> |
```

## changes/_template.md

```markdown
# <Change title>

Status: draft | in progress | done
Group: <Jidoka group id, once created>

## Goal

<What changes and why, in a few sentences.>

## Repositories and pull requests

| Repository | Pull request | State | Waits on |
| --- | --- | --- | --- |
| <name> | <link, when opened> | open, merged | <pull request or milestone> |

## Milestones

| Milestone | Status | Evidence |
| --- | --- | --- |
| <what must be true, such as "module v2.4.0 published"> | pending, confirmed | <link> |

## Decisions and open questions

- <decision, with the reason>

## Rollout notes

<Order of deployment, rollback, who to tell.>
```

## learnings/README.md

```markdown
# Learnings

One note per topic, named `<topic>.md`. Each fact carries its date and source (repository and pull request or file). Mark facts you are unsure are durable as `unverified`; a review promotes them to `platform/` or `recipes/`, or deletes them.
```

## recipes/README.md

```markdown
# Recipes

One note per common cross-repo change, named `<recipe>.md`: the steps in order, which repositories are touched, and the gotchas. Each recipe carries its date and the pull requests it was learned from.
```

## A learning note

```markdown
# <Topic>

- <Fact in one sentence.> (<date>, <repository>#<pull request> or <file>) <unverified, if so>
```
