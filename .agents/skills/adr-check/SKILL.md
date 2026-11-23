---
name: adr-check
description: Decide whether a change needs an ADR in docs/adr/ and write it when it does. Use before opening a pull request, or when a change sets a format, removes a feature or makes a hard-to-reverse choice.
---

# ADR check

Decide whether the change in the current worktree needs an ADR in `docs/adr/`, and write it when it does. The ADR belongs in the same pull request as the change, so the rejected alternatives are not lost.

1. Read the diff against the base branch (`git diff <base>...HEAD`) and the format in `skills/jidoka-domain-modeling/ADR-FORMAT.md`.

2. An ADR is needed when the change involves a real trade-off and is hard to reverse or surprising. In this project that usually means a change that:
   - sets or changes a format or contract between parts, such as the data folder layout, the dashboard API, tokens or the run record;
   - removes a command, feature, tracker or supported option;
   - sets a cap, default or routing choice that looks arbitrary;
   - overwrites or deletes user files;
   - is a breaking change on 0.x that a user will have to act on.

   Routine fixes and changes that are easy to reverse need none.

3. If a platform cannot be supported equally (GitHub and Azure DevOps, Claude, Codex and Copilot, Linux and Windows), discuss it with the user first and record an ADR before proceeding.

4. When one is needed, scan `docs/adr/` for the highest number and add `NNNN-short-slug.md` with the next number. Use the template in the format guide, add `Considered Options` and `Consequences` only when they add value, and set `status: accepted`. If the reason for the decision is unknown, use `status: proposed` and ask the user instead of guessing.

5. Report one line: the ADR you added (or the existing one the change follows), or `no ADR needed` with the reason. The pull request description must say which, so pass it on to `jidoka-pr`'s `**Decision:**` line.
