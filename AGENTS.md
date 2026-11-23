# Project instructions

The old project means [TDSchreur/tds-factory](https://github.com/TDSchreur/tds-factory).

Day-to-day pull requests target `develop`. Release pull requests merge `develop` into `master`; see [the release guide](docs/releasing.md).

## Always work through jidoka

Every change to this repository goes through the `jidoka` skill: features, bug fixes, docs, skills, scripts and config. Start with `jidoka` (or continue the run when given a run ID) and let it pick the workflow.

- "The change is small" is never a reason to skip jidoka. Small changes use `jidoka-fast`, which exists for exactly this: it keeps the run record and workspace without the Check stage.
- Do not edit files, commit or open a pull request outside a jidoka run. This applies to one-line fixes, typos and changelog-only edits too.
- Skip jidoka only when the user explicitly says so in this conversation (for example "do this without jidoka"). Your own judgment about size or urgency does not count.
- Pure questions and read-only investigation need no run.

## Rust

- Write idiomatic, safe Rust; avoid `unsafe` unless there is a clear need.
- Follow the binding [CODING_STANDARDS.md](CODING_STANDARDS.md) when writing or reviewing Rust.
- Handle errors with useful context, especially at user-facing and I/O boundaries.
- Use the [Rust skill](.agents/skills/rust/SKILL.md) for guidance when writing or reviewing Rust. Apply its recommendations with judgment: avoid abstractions that add complexity without a concrete benefit.
- Consult the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) when designing public APIs.
- Follow existing conventions and keep changes proportional to the task.

## Development principles

Jidoka is a small application. Keep it simple and fast to develop:

- Prefer straightforward code that a junior developer can understand.
- Follow existing conventions unless there is a good reason to change them.
- Optimize for readers: make intent clear and avoid clever abstractions.
- Keep related concerns together; split modules or services when there is a concrete need.
- Avoid unnecessary work and complexity before reaching for performance optimizations.
- Add dependencies only when they solve a real need.
- Move fast and try new ideas. Breaking changes are acceptable while the project is on 0.x.
- Do not use TDD. Write only the minimum tests needed to cover new or fixed behavior, and skip tests for exploratory work.
- Do not add compatibility layers or migrations for breaking changes; mention them in the changelog fragment.

## Parity rules

- GitHub and Azure DevOps get the same level of support. When something cannot be implemented for one of them, discuss it with the user and record an ADR in `docs/adr/` before proceeding.
- Claude, Codex and Copilot get the same level of support. When something cannot be implemented for one of them, discuss it with the user and record an ADR in `docs/adr/` before proceeding.
- Linux and Windows get the same level of support; macOS is not supported ([ADR-0008](docs/adr/0008-linux-and-windows-are-supported-equally-macos-is-not.md)). When something cannot be implemented for one of them, discuss it with the user and record an ADR in `docs/adr/` before proceeding.

## Architecture decision records

Record decisions in `docs/adr/` (format: [ADR-FORMAT.md](skills/jidoka-domain-modeling/ADR-FORMAT.md)), in the same pull request that makes them, so the rejected alternatives are not lost. Write an ADR when a change involves a real trade-off and is hard to reverse or surprising. In this project that usually means a change that:

- sets or changes a format or contract between parts, such as the data folder layout, the dashboard API, tokens or the run record;
- removes a command, feature, tracker or supported option;
- sets a cap, default or routing choice that looks arbitrary;
- overwrites or deletes user files;
- is a breaking change on 0.x that a user will have to act on.

Skip it for routine fixes and changes that are easy to reverse. If the reason for a decision is unknown, write the ADR with status `proposed` and ask the user instead of guessing. Say in the pull request description which ADR the change adds or follows, or that none was needed.

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

GitHub pull requests that only touch ordinary Markdown documentation run a changed-file check. This covers Markdown in `docs/` and `changelog.d/`, plus `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `GLOSSARY.md`, `README.md`, `SECURITY.md` and `SUPPORT.md`. Markdown in the bundled skill directories listed in `build.rs` or under `stages/` also runs the Linux Rust tests. Other pull requests and pushes to `develop` and `master` run full CI. Pull requests also run full CI when the base commit has no classifier. The branch rulesets require `ci-ok` on pull requests into `develop` and `master`.

While iterating and when finishing a ticket, run `scripts/verify.sh --quick`: it skips the Windows clippy step and runs the dashboard browser tests only when the dashboard or its tests changed. Run `scripts/verify.sh` without `--quick` before opening a pull request; it runs the same checks as CI, in the same order. A missing tool (`rustfmt`, `clippy`, `python3`, Chromium) is a blocker to install (`rustup component add rustfmt clippy` for the Rust ones), not something to skip and leave to CI. The one exception is the dashboard browser tests: `scripts/verify.sh` skips them on WSL2 and with `JIDOKA_SKIP_BROWSER_TESTS=1`, and you must say in the pull request that they were skipped. These instructions are informed by [Awesome Copilot's Rust instructions](https://github.com/github/awesome-copilot/blob/main/instructions/rust.instructions.md).

## Agent skills

### Issue tracker

Specs and tickets are tracked in GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

Triage uses the default role names as GitHub labels. See `docs/agents/triage-labels.md`.

### Domain docs

This repository uses a single-context domain layout. See `docs/agents/domain.md`.
