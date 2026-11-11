# Project instructions

Keep Jidoka simple and fast to develop. Prefer straightforward solutions and add dependencies only when they solve a real need.

The old project means [TDSchreur/tds-factory](https://github.com/TDSchreur/tds-factory).

Day-to-day pull requests target `develop`. Release pull requests merge `develop` into `master`; see [the release guide](docs/releasing.md).

## Rust

- Write idiomatic, safe Rust; avoid `unsafe` unless there is a clear need.
- Follow the binding [CODING_STANDARDS.md](CODING_STANDARDS.md) when writing or reviewing Rust.
- Handle errors with useful context, especially at user-facing and I/O boundaries.
- Use the [Rust skill](.agents/skills/rust/SKILL.md) for guidance when writing or reviewing Rust. Apply its recommendations with judgment: avoid abstractions that add complexity without a concrete benefit.
- Consult the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) when designing public APIs.
- Follow existing conventions and keep changes proportional to the task. TDD is not required.

## Move fast

Jidoka is not a big enterprise application. Move fast, try new ideas, and breaking things is fine, especially while the project is on 0.x.

- Do not use TDD. Write only the minimum tests needed to cover new or fixed behavior, and skip tests for exploratory work.
- Do not add compatibility layers or migrations for breaking changes; mention them in the changelog fragment.

## Parity rules

- GitHub and Azure DevOps get the same level of support. When something cannot be implemented for one of them, discuss it with the user and record an ADR in `docs/adr/` before proceeding.
- Claude, Codex and Copilot get the same level of support. When something cannot be implemented for one of them, discuss it with the user and record an ADR in `docs/adr/` before proceeding.

## Shipped skills are project-agnostic

The skills that `jidoka install` bundles (listed in `build.rs`) are used by anyone, on any kind of project. They must not name this repository's scripts, branches, tools or conventions, such as `scripts/verify.sh` or `develop`. Put those in `AGENTS.md`, the README or `.github/instructions/`, and word the skill generically, for example "the project's verification" or "the base branch chosen in Workspace". A test in `src/install.rs` fails when a bundled skill names this repository; search the skills you changed for this repository's names before opening a pull request.

## Scoped guides

Read the relevant guide when working on these files or tasks:

- [Reliable tests](.github/instructions/reliable-tests.instructions.md): tests involving time, shared state, files, ports, or processes.
- [External tools](.github/instructions/external-tools.instructions.md): starting `az`, `gh`, `git` or other programs, which must also work on Windows.
- [Skill authoring](.github/instructions/agent-skills.instructions.md): skills maintained in this project, including how to check a skill change.
- [Dashboard accessibility](.github/instructions/accessibility.instructions.md): dashboard HTML, CSS, and interactive behavior.
- [Shell scripts](.github/instructions/shell.instructions.md): `*.sh` files, including the Linux installer.
- [PowerShell scripts](.github/instructions/powershell.instructions.md): `*.ps1` files, including the Windows installer.
- [Docs and changelog](.github/instructions/update-docs-on-code-change.instructions.md): code changes that need documentation or a `changelog.d` fragment.
- [Markdown](.github/instructions/markdown.instructions.md): Markdown written or edited in this project.
- [GitHub Actions](.github/instructions/github-actions-ci-cd-best-practices.instructions.md): GitHub Actions workflows. Use the [github-actions-hardening skill](skills/github-actions-hardening/SKILL.md) to review workflow security.

## Commands

- Format: `cargo fmt --check`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Test: `cargo test`
- Dashboard browser tests: `python3 tests/dashboard-<name>.py` for every `tests/dashboard-*.py` (needs `python3` and Chromium or Google Chrome; they share `tests/dashboard_browser.py`, written for Linux and macOS and run on Linux only in CI)

After a pull request merges, finish from the main checkout so local agents always use the latest jidoka:

1. `jidoka run cleanup <run-id>` removes the run's worktree and local branch.
2. `JIDOKA_AGENT=all scripts/update-local.sh` pulls `develop`, rebuilds, reinstalls the `jidoka` command and the skills for all agents, and restarts the dashboard.

While iterating and when finishing a ticket, run `scripts/verify.sh --quick`: it skips the Windows clippy step and runs the dashboard browser tests only when the dashboard or its tests changed. Run `scripts/verify.sh` without `--quick` before opening a pull request; it runs the same checks as CI, in the same order. A missing tool (`rustfmt`, `clippy`, `python3`, Chromium) is a blocker to install (`rustup component add rustfmt clippy` for the Rust ones), not something to skip and leave to CI. These instructions are informed by [Awesome Copilot's Rust instructions](https://github.com/github/awesome-copilot/blob/main/instructions/rust.instructions.md).

## Agent skills

### Issue tracker

Specs and tickets are tracked in GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

Triage uses the default role names as GitHub labels. See `docs/agents/triage-labels.md`.

### Domain docs

This repository uses a single-context domain layout. See `docs/agents/domain.md`.
