# TDS Factory

An early Rust foundation for a small software factory, inspired by the original [TDS Factory](https://github.com/TDSchreur/tds-factory). The goal is to take a piece of work from idea to delivery with a simple flow and low time and token costs.

The core flow is **define → build → check → finish**, implemented as an agent skill. Start small and add steps only when they are useful.

## Getting started

Install a Rust toolchain with Cargo, then run:

```sh
cargo run -- --help
cargo run -- --version
cargo run -- dashboard
```

The dashboard is available at <http://127.0.0.1:4311>; press Ctrl+C to stop it. The Rust server stores run progress under `$XDG_DATA_HOME/tds-factory2/` (or `~/.local/share/tds-factory2/` when unset). Keep it running while using factory2. Install the CLI with `cargo install --path . --root ~/.local` so the skill can call `factory2` from any project (ensure `~/.local/bin` is on PATH). Restart a previously running dashboard after upgrading.

For manual tracking, `factory2 run start "Task title"` prints a run ID, and `factory2 run update <id> build running` updates it. Stages are `define`, `build`, `check`, and `finish`; statuses are `running`, `blocked`, and `done`.

The server listens only on localhost but has no user authentication. Other users on the same machine can read and change run titles and project paths; use it only on a trusted machine.

## Skills

The [`factory2` skill](skills/factory2/SKILL.md) coordinates the flow using skills from [mattpocock/skills](https://github.com/mattpocock/skills) and [github/awesome-copilot](https://github.com/github/awesome-copilot). To start using it, point your coding agent to `skills/factory2/SKILL.md` in this checkout and ask it to run factory2 on your task. The CLI does not install or invoke skills yet. See [skills/README.md](skills/README.md) for the upstream origins.

The [Rust skill](skills/rust/SKILL.md) and [project instructions](AGENTS.md) guide Rust development in this repository.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
