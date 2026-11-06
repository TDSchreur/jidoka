# One verify command

## Problem and outcome

PR #49 failed CI on `cargo fmt --check`, a check nobody ran locally: the instructions said to run "the relevant checks", `rustfmt` and `clippy` were not installed, and the gap was left for CI. Add one command that runs what CI runs, and make the contributor instructions and the bundled workflow skills require it, so a missing tool or a failing check blocks the work instead of being skipped.

## Behavior

- `scripts/verify.sh` runs, in this order and stopping at the first failure, the same steps as the `check` job in `.github/workflows/ci.yml`: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`. It prints the step it is running and ends with an all-passed line.
- Before running anything it checks that `cargo`, `rustfmt` and `clippy` are available. If one is missing it fails, names the missing tool and says how to install it (`rustup component add rustfmt clippy`). It never skips a check.
- `scripts/verify.sh` and `ci.yml` each carry a comment saying they must stay in sync.
- `AGENTS.md` tells contributors and agents to run `scripts/verify.sh` before finishing any ticket or opening a PR, and that a missing tool is a blocker to fix, not something to leave to CI. The existing Commands list stays.
- The `factory-full`, `factory-balanced` and `factory-fast` skills, and `implement-spec-balanced` and `implement-spec-fast`, require running the project's verification (this repo's is `scripts/verify.sh`; the generic wording is in `project-agnostic-skills.md`) (after each ticket in Build for `factory-full` and `factory-balanced`, before Finish for `factory-fast`, and on the final commit before Finish for all three), and treat a missing tool or failing check as blocking: fix it, or mark the run blocked, never skip. The upstream `implement-spec` skill is left unchanged.
- The README Development section and a `changelog.d` fragment mention the command.

## Verification

Behavior tests run the script against a fake `cargo` on `PATH`, so no real checks execute: the three checks run in order and success is reported; a failing check stops the later ones; a missing `cargo`, `rustfmt` or `clippy` fails with the install instruction and runs no check. Run `scripts/verify.sh` itself before opening the PR.

## Tickets

1. `scripts/verify.sh` with its behavior tests and the sync comments.
2. `AGENTS.md` rule and the README Development section.
3. Workflow skills and the two local `implement-spec-*` skills, plus the changelog fragment. Depends on ticket 1 for the command name.
