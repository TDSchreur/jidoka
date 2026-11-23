---
name: verify-and-report
description: Run the project's verification at the right depth and report the result, including skipped checks. Use when finishing a change, finishing a ticket or before opening a pull request.
disable-model-invocation: true
argument-hint: "[quick|full]"
---

# Verify and report

Run from the worktree of the change, not the main checkout. If the user gave `quick` or `full`, use it. Otherwise choose:

- `quick` while iterating and when finishing a ticket.
- `full` before opening a pull request, and on the final commit of a run.

1. Run the check:

   ```sh
   scripts/verify.sh --quick   # or: scripts/verify.sh
   ```

   It stops at the first failing step. Quick mode skips the Windows clippy step and runs the dashboard browser tests only when the dashboard or its tests changed.

2. Report the outcome. For a failure, name the failing step and show its output. Do not fix anything unless the user asked.

3. A missing tool (`rustfmt`, `clippy`, `python3`, Chromium) is a blocker to install, not something to skip. Install `rustfmt` and `clippy` with `rustup component add rustfmt clippy`.

4. Say which parts did not run, using the script's `Skipped` lines:
   - Dashboard browser tests are skipped on WSL2, with `JIDOKA_SKIP_BROWSER_TESTS=1`, and in quick mode without dashboard changes. Never claim they passed when they were skipped. In a pull request, say they were skipped.
   - Windows clippy is skipped in quick mode and when no Windows target is installed (`rustup target add x86_64-pc-windows-gnu`).

End with one line: the mode that ran and `passed`, `failed at <step>` or `blocked: <tool>`, plus anything skipped.
