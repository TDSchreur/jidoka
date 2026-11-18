# Getting started with several repositories

A platform change often needs pull requests in several repositories. This guide gets you from nothing to a first change across them, using two skills that ship with Jidoka. They work the same with Claude, Codex and Copilot, on GitHub and Azure DevOps, on Linux and Windows. How groups behave is in [Changes across repositories](multi-repo.md).

## The knowledge repo

A **knowledge repo** is an ordinary repository, one per user, that holds:

- `platform/`: how the repositories relate, the list of repositories (name, host, remote, role, owner, default branch) and the environments.
- `changes/<name>.md`: one spec per cross-repo change.
- `learnings/` and `recipes/`: facts agents discovered, and how to do common changes.

It is optional; change groups work without it. It keeps no local checkout paths, because those differ per person and machine. They go in `knowledge.md` in Jidoka's config folder (`~/.config/jidoka/` on Linux, `%APPDATA%\jidoka\` on Windows). This file is interim: a later release replaces it with a setting.

The knowledge repo is not worked through Jidoka runs. Agents pull with rebase, commit one note, and push directly to its default branch, because the updates are small and frequent.

## Steps

1. **Create the repository** on your host, empty, and clone it.
2. **Run `/jidoka-setup-knowledge-repo`.** It asks which repositories to include, where each is hosted and its remote, and where each is checked out, then writes the layout and pushes it.
3. **Start a change with `/jidoka-cross-repo-change`.** It reads `platform/`, writes `changes/<name>.md` and pushes it, finds the touched repositories with you, and asks you to start one ordinary run per repository, each in a fresh session in that checkout. It then creates the group with milestones and dependencies. It never implements code across repositories itself.
4. **Keep the spec current** as pull requests open and milestones are confirmed, and close out with `jidoka group inspect <group-id>`.

## Self-learning

Agents record durable platform facts they discover in the knowledge repo: facts not already there, one per note, with date and source, never secrets or personal data. Unsure facts go in `learnings/` marked unverified. **For now this only happens while one of the two skills is in use**, because nothing else tells agents about the knowledge repo yet.
