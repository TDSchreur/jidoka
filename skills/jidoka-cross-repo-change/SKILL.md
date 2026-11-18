---
name: jidoka-cross-repo-change
description: "Guide one change that spans several repositories: read the platform knowledge, write a shared change spec, find the touched repositories with the user, start one ordinary Jidoka run per repository, link the runs in a change group with milestones and dependencies, and close out. Use whenever a change needs pull requests in more than one repository, for example a module plus the infrastructure and API that consume it, or when the user asks to coordinate, group or plan work across repositories, even if they never say 'change group' or 'knowledge repo'."
---

# Change across repositories

This skill coordinates; it never implements code across repositories itself. Each repository gets its own ordinary Jidoka run, in its own checkout and fresh agent session, with its own worktree, gates and pull request. A **change group** only links those runs to one shared spec and records what each waits on.

The knowledge repo is optional and is not worked through Jidoka runs: pull with rebase, make one change, commit it alone, push directly to its default branch, and on a rejected push pull again and retry. Group commands need a running dashboard (`jidoka dashboard`).

## 1. Find the knowledge repo

Read `knowledge.md` in Jidoka's per-user config folder (`~/.config/jidoka/` on Linux, `%APPDATA%\jidoka\` on Windows). It holds a `Path:` line with the knowledge repo checkout and a two-column table (Repository, Checkout) of repository name to checkout path. If it is missing, offer `jidoka-setup-knowledge-repo`; if the user declines, continue with a spec in a place they name (a tracker issue or a document with a web link) and skip the knowledge steps.

Pull with rebase, then read `platform/` (overview, repositories, environments) and search `recipes/` and `learnings/` for the topic (skip folders that do not exist). This tells you which repositories are likely touched, their order and conventions.

## 2. Write the spec

Copy `changes/_template.md` from the knowledge repo to `changes/<name>.md` (short kebab-case name; if the repo has no template, use [references/change-spec.md](references/change-spec.md)). Fill in the goal and, with the user, the repositories, expected pull requests and milestones. A milestone is something merging does not prove, such as "module v2.4.0 published" or "pipeline deployed to test". Commit and push it alone.

Take its **web link** from the repository host, not a local path: reviewers open the URL. Build it from the remote, using the default branch:

- GitHub: `https://github.com/<owner>/<repo>/blob/<branch>/changes/<name>.md`
- Azure DevOps: `https://dev.azure.com/<org>/<project>/_git/<repo>?path=/changes/<name>.md&version=GB<branch>`

Read the remote with `git remote get-url origin`, strip a trailing `.git`, and convert an SSH remote (`git@github.com:<owner>/<repo>`, `git@ssh.dev.azure.com:v3/<org>/<project>/<repo>`) to the HTTPS form above. Use the repository's real default branch. Check the file is pushed (`git ls-remote origin <branch>` shows the pushed commit, and the file is in it) before using the link.

## 3. Find the touched repositories

Agree the list with the user, starting from `platform/repos.md`. For each, find its checkout in `knowledge.md`; ask when it is unknown or is not a git checkout of that repository, and add or correct the row there. Say which repositories you could not place, and do not guess a path.

## 4. Start one run per repository

For each repository, tell the user to start a run in a fresh agent session inside that checkout, through the `jidoka` skill, with a prompt that names the spec link and that repository's part of the change. The spec link must exist before the runs start, so they can name it. Do not run several repositories from this session. Keep the run IDs the user reports back. A run can also be attached later, once you learn the change touches another repository.

## 5. Create the group

```bash
jidoka group create "<change title>" --spec <spec web link>
jidoka group add <group-id> <run-id>
jidoka group milestone add <group-id> "<milestone text>"
jidoka group depend <group-id> <run-id> --milestone <milestone-id>
jidoka group depend <group-id> <run-id> --on-pr <upstream-run-id>
```

`create` and `milestone add` print the group and milestone IDs; keep them for the later commands. Use `--run <id>` on `create` for the first run. Dependencies are advisory: nothing blocks, and each run's agent describes its prerequisites in the pull request. Afterwards add the group ID to the spec's `Group:` line and its milestone IDs next to the milestones, as a second small commit. The spec is the team's view; the group is only the local record, so keep both in step. If a group command says no dashboard is running, start `jidoka dashboard` in the background and retry once.

## 6. Keep the spec current

Jidoka does not edit the spec. Remind the user, and do it for them when asked: whoever opens or replaces a pull request adds its link; whoever confirms or reopens a milestone updates its status and evidence (`jidoka group confirm <group-id> <milestone-id> --evidence <url>`, `jidoka group reopen`). Each pull request links the spec. Pull request states are refreshed while the dashboard runs, or on demand with `jidoka run sync <run-id>`.

## 7. Close out

Run `jidoka group inspect <group-id>` and report the members, pull request states and pending milestones. When all pull requests are merged and milestones confirmed, set the spec's status to done and record what was learned (below). Detach runs and delete the group only if the user asks.

## Self-learning rules

Record durable platform facts you discover while coordinating, in the knowledge repo.

- Record a **durable fact about the platform** that was not already in the knowledge repo: how repositories relate, deployment order, naming, conventions, gotchas, ownership. Do not record task status, speculation, or facts about one repository that its own docs already hold.
- Read first. Search the knowledge repo before writing. Update an existing note instead of adding a duplicate, and correct a note that turned out wrong.
- One fact per note or section, with the date and its source (repository and pull request or file). Keep it short.
- Never record secrets, tokens, credentials, personal data or private customer data.
- Commit and push as described at the top.
- When unsure that a fact is durable, put it in `learnings/` marked `unverified`. A later review promotes it to `platform/` or `recipes/`, or deletes it.

Until Jidoka itself tells agents about the knowledge repo, this applies only while this skill or `jidoka-setup-knowledge-repo` is in use.
