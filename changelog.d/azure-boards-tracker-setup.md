## Added

- `/setup-matt-pocock-skills` can set up Azure Boards as the issue tracker, for a repository on any host, including GitHub. It proposes Azure Boards when the remote is on Azure Repos, checks that `az`, the `azure-devops` extension, the sign-in and the project work, and has you confirm the project's work item types and states instead of assuming a process. When something is missing, it stops and says what, without writing a partial `docs/agents/issue-tracker.md`. Triage labels become work item tags.
