---
name: jidoka-setup-knowledge-repo
description: "Create or fill a knowledge repo: one ordinary repository per user that holds a platform's shared knowledge (which repositories exist, how they relate, conventions) and the specs of changes that touch several repositories. Interviews the user about their repositories, hosts and checkouts, then writes the layout, commits and pushes it. Use whenever the user wants to set up, start, add to or repair a platform knowledge repo, a docs repo for cross-repo changes, or a list of their repositories and where they are checked out, even if they do not say 'knowledge repo'."
---

# Set up a knowledge repo

A knowledge repo is an ordinary repository, one per user, with the platform's shared knowledge and the specs of cross-repo changes. Agents read it before a change and add what they learn. Jidoka change groups work without it; it only gives a new user a place to start.

The knowledge repo is **not** worked through Jidoka runs. Updates are small and frequent, so a run, worktree and pull request per note would make it too costly to keep up. Commit and push directly to its default branch, following [Writing to the knowledge repo](#writing-to-the-knowledge-repo).

## Interview

Ask one group of questions at a time and wait for the answers. Read what the user already has first: an existing knowledge repo checkout, `jidoka knowledge show` (see Local setting), and the remotes of repositories they name (`git remote get-url origin`). Do not ask what you can read.

1. **The knowledge repo itself.** What is the platform called (used in the README)? Does the repo exist? If yes, where is it checked out? If not, where should the new checkout go, and what is the remote (an empty repository the user has created on GitHub or Azure DevOps)? Never create a hosted repository for them.
2. **The repositories.** Which repositories make up the platform? For each: a short name, where it is hosted (GitHub or Azure DevOps), its remote URL, its role (module, infrastructure, API, pipeline and so on), its owner (a team or person) and its default branch. Take host and remote from the URL when the user gives one: `github.com` is GitHub, `dev.azure.com` or `visualstudio.com` is Azure DevOps.
3. **Checkouts.** Where is each repository checked out on this machine? Verify each path exists and its `origin` matches the remote; say so when it does not instead of correcting it silently.
4. **Overview.** In a few sentences: how do the repositories relate, in what order do changes ship, which environments exist? Leave a section as a line `TODO: <what is missing>` rather than inventing content.

## Write the layout

Use the templates in [references/templates.md](references/templates.md). Create only what is missing; never overwrite an existing note without showing the user the difference.

```text
README.md
platform/overview.md
platform/repos.md
platform/environments.md
changes/_template.md
learnings/
recipes/
```

Git does not track empty folders: put a short `README.md` in `learnings/` and `recipes/` (templates for each are in the references).

**Never put local checkout paths, secrets, tokens or personal data in the repo.** `platform/repos.md` lists name, host, remote, role, owner and default branch only. Paths differ per person and machine, and committing them breaks teammates.

## Local setting

Record the machine-local facts with Jidoka, never in the repository. They live in a per-user setting that Jidoka keeps in its config folder; do not edit the file by hand.

```bash
jidoka knowledge set <path of the knowledge repo checkout>
jidoka knowledge checkout <repository name> <path of its checkout>
jidoka knowledge show
```

Use one `checkout` call per repository, and run `show` at the end to confirm. The knowledge repo's own remote is not recorded; it is in the checkout's `origin`. Setting the knowledge repo is also what makes `jidoka run next` tell agents in every run to read it in Define and record learnings in Finish.

## Writing to the knowledge repo

Concurrent agents in several repositories may write here, so keep each write small:

1. `git pull --rebase` in the knowledge repo checkout (skip this on a brand-new empty clone, and push the first commit with `git push -u origin <default branch>`).
2. Make one change: one note, or one section of one note.
3. Commit it alone with a clear message, such as `platform: add deployment order for infra and api`.
4. Push to the default branch. If the push is rejected, pull with rebase again and retry; resolve a real conflict by reading both sides, never by discarding the other agent's text.

Use the plain `git` the user has. It talks to GitHub and Azure DevOps alike, so no host tool is needed for this.

## Self-learning rules

Apply these whenever you work with a knowledge repo, including during this setup.

- Record a **durable fact about the platform** that was not already in the knowledge repo: how repositories relate, deployment order, naming, conventions, gotchas, ownership. Do not record task status, speculation, or facts about one repository that its own docs already hold.
- Read first. Search the knowledge repo before writing. Update an existing note instead of adding a duplicate, and correct a note that turned out wrong.
- One fact per note or section, with the date and its source (repository and pull request or file). Keep it short.
- Never record secrets, tokens, credentials, personal data or private customer data.
- Commit and push as described above.
- When unsure that a fact is durable, put it in `learnings/` marked `unverified`. A later review promotes it to `platform/` or `recipes/`, or deletes it.

## Finish

Commit and push the layout (one commit per file is fine), then tell the user: the knowledge repo's path and remote, the repositories recorded, the output of `jidoka knowledge show`, and what is still a TODO. Point them to `jidoka-cross-repo-change` for the first change that spans repositories.
