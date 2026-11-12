---
name: jidoka-setup
description: "Configure this repo for the engineering skills: set up its issue tracker, triage label vocabulary, domain doc layout, and verification commands. Run once before first use of the other engineering skills."
disable-model-invocation: true
---

# Setup Jidoka

Scaffold the per-repo configuration that the engineering skills assume:

- **Issue tracker**: where issues live (GitHub by default; Azure Boards and local markdown are the other two)
- **Triage labels**: the strings used for the five canonical triage roles
- **Domain docs**: where `GLOSSARY.md` and ADRs live, and the consumer rules for reading them
- **Verification**: the project checks to run before merging

This is a prompt-driven skill, not a deterministic script. Explore, present what you found, confirm with the user, then write.

## Process

### 1. Explore

Look at the current repo to understand its starting state. Read whatever exists; don't assume:

- `git remote -v` and `.git/config`: is this a GitHub repo? Which one? A remote on `dev.azure.com`, `ssh.dev.azure.com` or `*.visualstudio.com` is an Azure Repos repo.
- `AGENTS.md` and `CLAUDE.md` at the repo root: does either exist? Is there already an `## Agent skills` section in either?
- `GLOSSARY.md` and `GLOSSARY-MAP.md` at the repo root, and the legacy `CONTEXT.md` / `CONTEXT-MAP.md`. If a legacy file exists, offer to `git mv` it to its `GLOSSARY` name and rewrite those names in `docs/agents/domain.md`.
- `docs/adr/` and any `src/*/docs/adr/` directories
- `docs/agents/`: does this skill's prior output already exist?
- `.scratch/`: a sign that a local-markdown issue tracker convention is already in use
- Is the `jidoka-triage` skill installed? (a `jidoka-triage` skill folder alongside this one, or `jidoka-triage` in your available skills.) This decides whether Section B runs at all.
- Verification instructions in agent instructions, contributing docs, build files and CI: which commands check formatting, linting, tests and other required checks? Distinguish quick checks from the full pre-merge verification.
- Monorepo signals: a `pnpm-workspace.yaml`, a `workspaces` field in `package.json`, or a populated `packages/*` with its own `src/`. These are present only in a genuinely large multi-package repo; their absence means single-context, which is almost every repo.

### 2. Present findings and ask

Summarise what's present and what's missing. Then take the sections in order. One section, one answer, then the next.

Lead each section with the recommended answer so the user can accept it in a word. Give a one-line explainer only when the choice genuinely branches; skip the section entirely when exploration already settled it (Section B when `jidoka-triage` isn't installed, Section C when there's no monorepo).

**Section A: Issue tracker.**

> Explainer: The "issue tracker" is where issues live for this repo. Skills like `jidoka-to-tickets`, `jidoka-triage`, and `jidoka-to-spec` read from and write to it. They need to know whether to call `gh issue create`, or write a markdown file under `.scratch/`. Pick the place you actually track work for this repo.

Default posture: these skills were designed for GitHub. If a `git remote` points at GitHub, propose that. If a `git remote` points at Azure Repos, propose Azure Boards. Otherwise (or if the user prefers), offer these three, the only supported trackers:

- **GitHub**: issues live in the repo's GitHub Issues (uses the `gh` CLI)
- **Azure Boards**: issues live as work items in an Azure DevOps project (uses the `az` CLI with the `azure-devops` extension). Available for any repository host, including GitHub repos.
- **Local markdown**: issues live as files under `.scratch/<feature>/` in this repo (good for solo projects or repos without a remote)

The tracker is independent of where the code is hosted: a GitHub repo can track its work in Azure Boards, and an Azure Repos repo can use any of the three.

Record the choice in `docs/agents/issue-tracker.md`. The GitHub template carries a "PRs as a request surface" flag, defaulted **off**. Leave it off and don't raise it: a user who wants external PRs in the triage queue can flip the flag in the file later. The Azure Boards template fixes this flag at **no**, because pull requests are not work items.

A GitHub tracker needs a reachable repo. When the user picks one and no `git remote` points at that host, get the repo URL from them and wire it up (`git init` if needed, `git remote add origin <url>`) before going further. Done when `gh repo view` prints the project from inside this directory; if it can't, stop and tell the user what's missing.

An Azure Boards tracker needs a reachable project and its process details. When the user picks it:

1. Ask for the organization URL (such as `https://dev.azure.com/<organization>`) and the project name.
2. Check the tooling and access in order. Run `az version`; if it fails, tell the user to install or fix the Azure CLI. Run `az extension show --name azure-devops`; if it fails, tell the user to install the Azure DevOps extension with `az extension add --name azure-devops`. Run `az devops invoke --org <org-url> --detect false --area wit --resource workitemtypes --route-parameters "project=<project>" --api-version 7.1`; if it fails, tell the user to check sign-in (`az login`, `az devops login` or `AZURE_DEVOPS_EXT_PAT`) and that the organization and project are correct and visible to them. Done when the work item types list prints. Never print a token.
3. From that list, propose a work item type for specs, one for tickets and one for bugs, and let the user confirm or change them. Don't assume the names of the Agile, Scrum, Basic or CMMI process; use what the project returns.
4. For each chosen type, read its states and their categories with `az devops invoke --org <org-url> --detect false --area wit --resource workitemtypestates --route-parameters "project=<project>" "type=<type>" --api-version 7.1`. Show the state per category (Proposed, InProgress, Completed, Removed) and propose the Removed state, or the Completed state when the type has none, as the "Wontfix state". Have the user confirm.
5. Fill in the configuration block of [issue-tracker-azure-boards.md](./issue-tracker-azure-boards.md) with these values.

If `az`, the extension, the sign-in, project access or any value of the configuration block is missing, stop and tell the user exactly what is missing and how to fix it. Do not write a partial `docs/agents/issue-tracker.md`.

**Section B: Triage label vocabulary.** Skip this section entirely if the `jidoka-triage` skill isn't installed (exploration told you), since an uninstalled skill needs no labels.

If it is installed, ask exactly one question:

> Do you want to keep the default triage labels? (recommended: **yes**)

The defaults are the five canonical roles, each label string equal to its name: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. On **yes**, write them as-is. Only if the user says no, usually because their tracker already uses other names (e.g. `bug:triage` for `needs-triage`), collect the overrides so `jidoka-triage` applies existing labels instead of creating duplicates.

**Section C: Domain docs.** Default to **single-context** (one `GLOSSARY.md` + `docs/adr/` at the repo root). This fits almost every repo; write it without asking.

Offer **multi-context** (a root `GLOSSARY-MAP.md` pointing to per-context `GLOSSARY.md` files) only when exploration found monorepo signals. Then confirm which layout they want.

**Section D: Verification commands.**

Present the checks found during exploration and recommend the complete set required before merging, including any relevant platform-specific checks. Ask the user to confirm or correct them. If none can be inferred, ask the user which commands to run; do not invent commands. If the user has no verification commands, explain that Doctor will report them missing and continue without creating an empty section.

### 3. Confirm and edit

Show the user a draft of:

- The agreed `## Commands` section or changes to it, if any
- The `## Agent skills` block to add to whichever of `CLAUDE.md` / `AGENTS.md` is being edited (see step 4 for selection rules)
- The contents of `docs/agents/issue-tracker.md`, `docs/agents/domain.md`, and `docs/agents/triage-labels.md` (the last only when `jidoka-triage` is installed)

Let them edit before writing.

### 4. Write

**Pick the file to edit:**

- If `CLAUDE.md` exists, edit it.
- Else if `AGENTS.md` exists, edit it.
- If neither exists, ask the user which one to create; don't pick for them.

Never create `AGENTS.md` when `CLAUDE.md` already exists (or vice versa); always edit the one that's already there.

If an `## Agent skills` block already exists in the chosen file, update its contents in-place rather than appending a duplicate. Don't overwrite user edits to the surrounding sections.

Record the agreed verification commands under `## Commands` in `AGENTS.md` if it exists, otherwise in the chosen instruction file. Doctor reads `AGENTS.md` first when both files exist. Put one command per bullet in inline code (for example, `- Test: ` followed by the command in backticks). Distinguish quick checks from full pre-merge verification when applicable. Preserve existing instructions and unrelated commands; update an existing section in place instead of adding a duplicate. Doctor reads command snippets in bullets under this heading, so keep explanatory inline code out of those bullets. If the user provided no commands, leave the section unchanged.

The block:

```markdown
## Agent skills

### Issue tracker

[one-line summary of where issues are tracked]. See `docs/agents/issue-tracker.md`.

### Triage labels

[one-line summary of the label vocabulary]. See `docs/agents/triage-labels.md`.

### Domain docs

[one-line summary of layout: "single-context" or "multi-context"]. See `docs/agents/domain.md`.
```

Include the `### Triage labels` sub-block, and write `docs/agents/triage-labels.md`, only when `jidoka-triage` is installed and Section B ran. When it isn't, both are omitted.

When Section B ran on GitHub, create each configured label the tracker lacks (`gh label create`). On Azure Boards the triage labels are work item tags, which are created the first time they are applied, so there is nothing to create.

Then write the docs files using the seed templates in this skill folder as a starting point:

- [issue-tracker-github.md](./issue-tracker-github.md): GitHub issue tracker
- [issue-tracker-azure-boards.md](./issue-tracker-azure-boards.md): Azure Boards issue tracker
- [issue-tracker-local.md](./issue-tracker-local.md): local-markdown issue tracker
- [triage-labels.md](./triage-labels.md): label mapping (only if `jidoka-triage` is installed)
- [domain.md](./domain.md): domain doc consumer rules + layout

### 5. Done

Tell the user the setup is complete, where the verification commands were recorded, and which engineering skills will now read from these files. If no commands were provided, mention that verification remains unconfigured. Mention they can edit `docs/agents/*.md` directly later; re-running this skill is only necessary if they want to switch issue trackers or restart from scratch.
