## Changed

- Lint rules from Microsoft's Pragmatic Rust Guidelines are enforced through `[lints]` in `Cargo.toml`: `#[allow]` attributes need a reason, `unwrap()` is not allowed outside tests, and unsafe blocks need a safety comment. No behavior change.
