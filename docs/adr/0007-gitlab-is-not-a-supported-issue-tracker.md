---
status: proposed
---

# GitLab is not a supported issue tracker

Setup used to offer GitLab and an "other tracker" option next to GitHub, Azure Boards and local Markdown. We decided that setup offers only GitHub, Azure Boards and local Markdown files in `.scratch/` ([PR #246](https://github.com/TDSchreur/jidoka/pull/246)). The PR, its commits and its changelog fragment do not say why GitLab was dropped, so this record is `proposed` until that reason is written down here.

<!-- Reason to fill in: why GitLab and the "other tracker" option were dropped. -->

## Consequences

- A repository whose `docs/agents/issue-tracker.md` chose GitLab or another tracker is no longer supported and needs a manual change. Nothing migrates it.
- GitHub and Azure DevOps stay at the same level of support, as the parity rules require. Adding a tracker means supporting it for every agent and recording an ADR if it cannot be done equally.
