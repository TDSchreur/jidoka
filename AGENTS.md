# Project instructions

Keep the factory simple and fast to develop. Prefer straightforward solutions and add dependencies only when they solve a real need.

## Rust

- Write idiomatic, safe Rust; avoid `unsafe` unless there is a clear need.
- Handle errors with useful context, especially at user-facing and I/O boundaries.
- Use the [Rust skill](skills/rust/SKILL.md) for guidance when writing or reviewing Rust. Apply its recommendations with judgment: avoid abstractions that add complexity without a concrete benefit.
- Consult the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) when designing public APIs.
- Follow existing conventions and keep changes proportional to the task. TDD is not required.

## Commands

- Format: `cargo fmt --check`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Test: `cargo test`

Run the checks relevant to the change. These instructions are informed by [Awesome Copilot's Rust instructions](https://github.com/github/awesome-copilot/blob/main/instructions/rust.instructions.md).
