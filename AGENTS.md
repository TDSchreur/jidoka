# Project instructions

Keep the factory simple and fast to develop. Prefer straightforward solutions and add dependencies only when they solve a real need.

The old project means [TDSchreur/tds-factory](https://github.com/TDSchreur/tds-factory).

Day-to-day pull requests target `develop`. Release pull requests merge `develop` into `master`; see [the release guide](docs/releasing.md).

## Rust

- Write idiomatic, safe Rust; avoid `unsafe` unless there is a clear need.
- Handle errors with useful context, especially at user-facing and I/O boundaries.
- Use the [Rust skill](skills/rust/SKILL.md) for guidance when writing or reviewing Rust. Apply its recommendations with judgment: avoid abstractions that add complexity without a concrete benefit.
- Use [rust-best-practices](.agents/skills/rust-best-practices/SKILL.md) for Rust work, applying its guidance with the same judgment.
- Consult the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) when designing public APIs.
- Follow existing conventions and keep changes proportional to the task. TDD is not required.

## Scoped guides

Read the relevant guide when working on these files or tasks:

- [Reliable tests](.github/instructions/reliable-tests.instructions.md): tests involving time, shared state, files, ports, or processes.
- [Skill authoring](.github/instructions/agent-skills.instructions.md): skills maintained in this project.
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

After a pull request merges, run `scripts/update-local.sh` from the main checkout to pull `develop`, rebuild, reinstall the skills, and restart the dashboard, so local agents always use the latest factory.

Run `scripts/verify.sh` before finishing any ticket and before opening a pull request. It runs the same checks as CI, in the same order. A missing tool (`rustfmt`, `clippy`) is a blocker to fix with `rustup component add rustfmt clippy`, not something to skip and leave to CI. These instructions are informed by [Awesome Copilot's Rust instructions](https://github.com/github/awesome-copilot/blob/main/instructions/rust.instructions.md).

## Agent skills

### Issue tracker

Specs and tickets are tracked in GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

Triage uses the default role names as GitHub labels. See `docs/agents/triage-labels.md`.

### Domain docs

This repository uses a single-context domain layout. See `docs/agents/domain.md`.
