---
name: changelog-fragment
description: Write a changelog.d fragment for a user-visible change, or say why none is needed. Use when finishing a change or before opening a pull request.
disable-model-invocation: true
argument-hint: "[issue-number]"
---

# Changelog fragment

Write a `changelog.d/` fragment for the change in the current worktree. Skip it for internal refactors and test-only changes; say so instead of writing one.

1. Read the diff against the base branch (`git diff <base>...HEAD`) and decide whether a user would notice the change: commands, flags, dashboard behavior, installers or bundled skills. Project-only skills and agents that `jidoka install` does not bundle are not user-visible.

2. Look at two or three recent fragments in `changelog.d/` and match their tone: one bullet per change, written for the user, in the present tense, with no run IDs or file paths unless the user must act on them.

3. Pick the file name. Use `<issue-number>.md` when there is an issue or pull request number, otherwise a short kebab-case slug such as `agent-attention.md`. Do not reuse an existing name.

4. Put each bullet under the heading that fits: `## Added`, `## Changed` or `## Fixed`. Use only the headings you need.

5. Breaking changes (0.x) get a `## Changed` bullet that says what a user must now do. There are no compatibility layers or migrations, so the fragment is the only notice. If the change is hard to reverse or surprising, check it for an ADR too (`adr-check`).

6. Show the user the fragment and its path. Do not commit unless the user asked or you are inside a jidoka run that says to.
