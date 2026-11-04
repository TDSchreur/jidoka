# TDS Factory

An early Rust foundation for a small software factory, inspired by the original [TDS Factory](https://github.com/TDSchreur/tds-factory). The goal is to take a piece of work from idea to delivery with a simple flow and low time and token costs.

The intended core flow is **define → build → check → deliver**. Its details are still to be decided; this repository does not implement the flow yet. Start small, add steps only when they are useful, and keep the human in control of delivery.

## Getting started

Install a Rust toolchain with Cargo, then run:

```sh
cargo run -- --help
cargo run -- --version
```

The CLI currently only displays help and its version. There are no runtime dependencies, agent integrations, or automated runs yet.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
