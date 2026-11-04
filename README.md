# TDS Factory

An early Rust foundation for a small software factory, inspired by the original [TDS Factory](https://github.com/TDSchreur/tds-factory). The goal is to take a piece of work from idea to delivery with a simple flow and low time and token costs.

The core flow is **define → build → check → finish**, implemented as an agent skill. Start small and add steps only when they are useful.

## Getting started

Once a release is published, install it with one command. No Factory 2 release has been published yet, so these commands are not available until then.

Linux x86-64:

```sh
curl -fsSL https://github.com/TDSchreur/tds-factory2/releases/latest/download/install.sh | sh
```

Windows x86-64 (PowerShell):

```powershell
irm https://github.com/TDSchreur/tds-factory2/releases/latest/download/install.ps1 | iex
```

Both installers download the prebuilt CLI, verify its release checksum, and install the factory2 skills for Codex. Set `FACTORY2_AGENT` to `claude` or `copilot` before running the installer to choose another agent. Existing skill directories are skipped, so edited or separately installed skills are not overwritten. To install skills for another agent later, run `factory2 install --agent claude` (or `codex` / `copilot`). To update a skill, first move its existing directory aside, then run the install command again. Re-running the one-line installer updates the CLI but also skips existing skills.

Stop the dashboard before upgrading, especially on Windows where a running executable cannot be replaced. The Windows CLI cross-compiles, but the installer has not yet been run on Windows; the release workflow tests the build on a Windows runner.

For development, install a Rust toolchain with Cargo, then run:

```sh
cargo run -- --help
cargo run -- --version
cargo run -- dashboard
```

The Rocket server binds to localhost at <http://127.0.0.1:4311>; press Ctrl+C to stop it. It stores run progress under `$XDG_DATA_HOME/tds-factory2/` (or `~/.local/share/tds-factory2/` when unset) on Linux, and `%LOCALAPPDATA%\tds-factory2\` on Windows. Keep it running while using factory2. To install from a checkout, use `cargo install --path . --locked`, ensure Cargo's bin directory is on PATH, and run `factory2 install --agent codex`. Restart a previously running dashboard after upgrading.

For manual tracking, `factory2 run start "Task title"` prints a run ID, and `factory2 run update <id> build running` updates it. Stages are `define`, `build`, `check`, and `finish`; statuses are `running`, `blocked`, and `done`.

The server listens only on localhost but has no user authentication. Other users on the same machine can read and change run titles and project paths; use it only on a trusted machine.

## Skills

The [`factory2` skill](skills/factory2/SKILL.md) coordinates the flow using skills from [mattpocock/skills](https://github.com/mattpocock/skills) and [github/awesome-copilot](https://github.com/github/awesome-copilot). After installing the skills, ask your coding agent to run factory2 on your task. See [skills/README.md](skills/README.md) for the upstream origins.

The [Rust skill](skills/rust/SKILL.md) and [project instructions](AGENTS.md) guide Rust development in this repository.

## Development

Day-to-day work targets `develop`; releases merge `develop` into `master`. See [the release guide](docs/releasing.md) for the release and branch-protection setup.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
