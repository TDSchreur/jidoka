# Coding standards

These Rust standards are binding for this application. The other entries in the [Microsoft Rust guidelines digest](.agents/skills/rust/references/microsoft-rust-guidelines.md) apply with judgment.

## Error handling (M-APP-ERROR)

- Use the standard library only. Do not add `anyhow` or `thiserror`.
- Return `Result<T, String>` from module functions. The message says what failed and includes context, for example `.map_err(|error| format!("git could not be started: {error}"))`.
- Use a small `pub(crate)` error enum only when a caller must branch on the cause, as with `DocumentError`, `BeginError` and `GhError`.
- Plain file and process I/O helpers may return `io::Result`.
- Return `Result<_, Box<dyn std::error::Error>>` from command entry points (`src/main.rs`, `src/client.rs`, `doctor::run`) so `?` converts both error kinds.
- Let `main` print the error to stderr and exit with failure. Write every message for the user.
- Do not introduce a second error approach.

## Panics (M-PANIC-IS-STOP, M-PANIC-ON-BUG, M-PANIC-MESSAGE)

- Panic only for bugs, never for recoverable errors. A panic stops the program.
- Give every intentional `panic!`, `assert!`, `unreachable!`, and `expect` a message with the reason and the relevant values.
- `Cargo.toml` enforces `clippy::unwrap_used` and `clippy::missing_panics_doc`. `clippy.toml` allows `unwrap` in tests.

## Constants (M-DOCUMENTED-MAGIC)

- Replace magic values with named constants.
- Document why the constant has that value.

## Documentation

- Describe the end state, not design history (M-NO-META-DESIGN-DOCUMENTATION).
- Write the first sentence of a doc comment on one line, at about 15 words (M-FIRST-DOC-SENTENCE).
- Add `# Errors` and `# Panics` sections where they apply (M-CANONICAL-DOCS).

## Tests (M-TAUTOLOGICAL-TESTS)

- Do not write tests that restate constants or mirror implementation branches.
- Test properties and observable behavior.

## Structure

- Split a module when it has a second reason to change, not when it reaches a line count.
- Name a module after its concept (as in GLOSSARY.md), not its kind (`utils`, `helpers`).
- Clippy enforces the function-level limits: 100 lines, cognitive complexity 25 and 6 arguments. Existing exceptions carry `#[expect]` with a reason; remove the `expect` when the function is fixed.

## Naming (M-WEASEL-WORDS)

- Avoid vague type names and suffixes such as `Manager`, `Service`, `Helper`, and `Factory`.
- Name a type after what it is or does.

## Guidelines that do not apply

- M-MIMALLOC-APPS: the default allocator is sufficient for this CLI.
- M-TARGET-CPU: the CLI ships to many machines, so it must not assume a specific CPU.
- M-SMALLER-CRATES, M-CARGO-WORKSPACE, M-CRATES-FLAT-FOLDER, M-CRATES-IN-WORKSPACE: this is a single binary crate with no workspace.
- M-ERRORS-CANONICAL-STRUCTS: there is no public library API, so no error types are exposed.
- M-INIT-BUILDER: there are no public library types to construct.
- M-IMPL-ASREF: there is no public library API whose parameters need to stay flexible.
- M-TYPES-SEND: there are no public library types that callers must move across threads.
- M-DONT-LEAK-TYPES: there is no public library API that could expose third-party types.
