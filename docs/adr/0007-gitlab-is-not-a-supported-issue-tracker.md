---
status: accepted
---

# GitLab is not a supported issue tracker

Setup used to offer GitLab and an "other tracker" option next to GitHub, Azure Boards and local Markdown. We decided that setup offers only GitHub, Azure Boards and local Markdown files in `.scratch/` ([PR #246](https://github.com/TDSchreur/jidoka/pull/246)). We cannot support every tool, so we support what the maintainer and the users actually use: GitHub and Azure DevOps as hosts, and Claude, Codex and Copilot as agents, each group at the same level ([parity rules](../../AGENTS.md)). Nobody uses GitLab, so it went. The "other tracker" option went for the same reason, since it promised support for any tool.

## Considered Options

- **Keep GitLab and "other tracker".** Rejected: nobody uses them, and a third host would have to be supported at the same level as GitHub and Azure DevOps.

## Consequences

- A repository whose `docs/agents/issue-tracker.md` chose GitLab or another tracker is no longer supported and needs a manual change. Nothing migrates it.
- The same rule applies to what comes next: a new host or agent is added only when someone uses it, and then at the same level as the others, with an ADR if it cannot be done equally.
