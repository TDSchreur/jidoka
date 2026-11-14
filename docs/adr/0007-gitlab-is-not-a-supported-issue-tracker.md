---
status: accepted
---

# GitLab is not a supported issue tracker

Setup used to offer GitLab and an "other tracker" option next to GitHub, Azure Boards and local Markdown. We decided that setup offers only GitHub, Azure Boards and local Markdown files in `.scratch/` ([PR #246](https://github.com/TDSchreur/jidoka/pull/246)). We cannot support every tool: the maintainer does not use GitLab, and none of the project's users do, so supporting it would be maintenance with no one to benefit. The "other tracker" option went for the same reason, since it promised support for any tool.

## Considered Options

- **Keep GitLab and "other tracker".** Rejected: nobody uses them. The supported hosts are GitHub and Azure DevOps, at the same level of support ([parity rules](../../AGENTS.md)), and a third host would have to match that level too.

## Consequences

- A repository whose `docs/agents/issue-tracker.md` chose GitLab or another tracker is no longer supported and needs a manual change. Nothing migrates it.
- GitHub and Azure DevOps stay at the same level of support, as the parity rules require. Adding another host means bringing it to that level, and an ADR if it cannot be done equally.
