# Contributing

Thanks for helping improve Jidoka. Bugs and ideas go to [GitHub Issues](https://github.com/TDSchreur/jidoka/issues); pull requests are welcome. Questions are covered in [SUPPORT.md](SUPPORT.md), and everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## Branches and releases

Day-to-day work targets `develop`; releases merge `develop` into `master`. See [the release guide](docs/releasing.md) for the release and branch-protection setup.

## Verify your change

Run `scripts/verify.sh` before finishing a change and opening a pull request. It runs the same checks as CI, in the same order, and stops at the first failure; it needs `cargo`, `rustfmt` and `clippy` (`rustup component add rustfmt clippy`), plus `python3` and Chromium or Google Chrome for the dashboard browser tests (set `CHROMIUM` to the browser path if it is not on `PATH` as `chromium`, `chromium-browser` or `google-chrome`). When a Windows target is installed (`x86_64-pc-windows-gnu` preferred), it also runs clippy for it after the Linux clippy step (a local extra: CI lints on Linux only); without one it prints how to add it (`rustup target add x86_64-pc-windows-gnu`) and continues. The individual commands are:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --locked --target x86_64-pc-windows-gnu -- -D warnings # only when that target is installed
cargo test --locked
for test in tests/dashboard-*.py; do python3 "$test"; done
```

## Guidelines

The [Rust skill](.agents/skills/rust/SKILL.md) and [project instructions](AGENTS.md) guide Rust development in this repository.

Add a `changelog.d` fragment for user-visible changes, and update the docs in `docs/` when a command, option or install step changes. See [the docs instructions](.github/instructions/update-docs-on-code-change.instructions.md).

## Development machine

To switch a Linux x86-64 development machine between the latest release and the current `origin/develop`, run `scripts/switch-jidoka.sh release` or `scripts/switch-jidoka.sh develop` from this checkout. The script fetches and builds `develop` in a temporary checkout without changing your working tree, stops the dashboard, removes the existing Cargo and `~/.local/bin` jidoka executables, and installs the selected version in `~/.local/bin`. It prompts for a comma-separated list of agents, or uses `JIDOKA_AGENT` when set. Selected agents receive the bundled skills for that version. Restart the dashboard afterwards. Ensure `~/.local/bin` is on your PATH.

## Dashboard front end

The dashboard page is plain files in `dashboard/` (`index.html`, `app.js`, `help.js`, `style.css`, `favicon.svg`), built with Preact and htm, committed in `dashboard/vendor/` and loaded through an import map, so there is no build step and no `npm install`. The server embeds the files in the binary and serves only the ones it lists under `/assets/`. When you add a file the page loads, add it to the list in `src/server.rs`. See [`dashboard/vendor/README.md`](dashboard/vendor/README.md) for the library versions; their licenses are in `third-party/`.
