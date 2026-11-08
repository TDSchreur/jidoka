## Added

- `scripts/verify.sh` also runs clippy for an installed Windows target (preferring `x86_64-pc-windows-gnu`), and otherwise prints how to add it. `factory doctor` gets a non-blocking `windows-lint` check for Rust projects on non-Windows machines with the same advice.
- `factory-code-review` carries a cross-platform checklist in its Standards axis for projects that target more than one platform, and `factory-procedures` requires the project's full verification, formatting included, before pushing a CI fix.
