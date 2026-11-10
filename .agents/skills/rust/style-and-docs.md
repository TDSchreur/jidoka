# Lints, Everyday Idioms, Documentation

## Lints and warnings

- rustfmt owns formatting, clippy owns idiom checks, the compiler owns memory safety. Spend attention on design.
- Fix a lint rather than silencing it. When a lint is genuinely wrong for one item, use `#[expect(clippy::name, reason = "...")]` on the narrowest item: `expect` warns when the lint stops firing, so stale suppressions get cleaned up. `#[allow]` only in generated code and macro output.
- Never put `#![deny(warnings)]` in source: new toolchains add warnings and break the build. Enforce warnings-as-errors in CI with `cargo clippy -- -D warnings`.
- Configure lints once in `Cargo.toml` (`[lints]` or `[workspace.lints]`), not with scattered crate attributes. Worth enabling: `unsafe_op_in_unsafe_fn`, `missing_debug_implementations`, the clippy groups at `priority = -1`, and restriction lints such as `undocumented_unsafe_blocks`, `allow_attributes_without_reason`, `map_err_ignore`. The suppression lints need `rust-version` 1.81 or later.

## Everyday idioms

- **Iterators over hand loops** when the pipeline is short and reads top-down. Collect fallible steps into `Result<Vec<_>, E>` and apply `?`. Keep an explicit `for` when the body is large, does several jobs, or needs awkward early returns. Bind a long chain to a name rather than putting it in a `for` header; prefer `for x in it` to `.for_each(|x| ..)`. See [references/iterators-over-indexing.md](references/iterators-over-indexing.md).
- **`Option`/`Result` combinators** (`map`, `and_then`, `ok_or_else`, `unwrap_or_else`, `as_ref`, `as_deref`) when clearer than `match`, and `match` when not. See [error-handling.md](error-handling.md).
- **`format!`/`write!`** with inline args (`format!("{name}: {count}")`) instead of chained `+` or `str::replace`; `write!` into an existing buffer in hot paths.
- **Explicit types where inference obscures intent**: on `.into()`, `.parse()`, `.collect()`, and complex iterator results. Prefer `let x: Vec<u32> = ..` to a turbofish.
- **Drop guards** for cleanup that must run on early return or unwind. Bind them to a name (`let _guard = ..`); `let _ = ..` drops immediately.
- **Closures capturing mixed ownership**: rebind in a block, then `move`:

  ```rust
  let job = {
      let cfg = cfg.clone();
      move || run(cfg, data)
  };
  ```

- **Freeze after setup**: build with `let mut`, then rebind `let data = data;` so later code cannot mutate it.
- **Imports**: one `use` per module path (`use foo::bar::{Bar, Baz};`); types and traits unqualified, free functions qualified by module (`mem::take`, `fs::read`). Import a trait inside the function that needs it unless the whole file uses it. No globs except `use super::*` in tests, `use Enum::*` in a function that matches on it, and a framework prelude such as `use bevy::prelude::*`.
- **Magic numbers** become named `const`s with a doc comment saying where the value comes from and what changing it affects.
- **Logging**: structured events through `tracing` or `log`, not `println!`/`dbg!`, except a CLI's own stdout output. No `format!` in event messages.

## Documentation

- `///` on every public item; `//!` at the top of every public module and the crate root. The first sentence is one line of about fifteen words, because it becomes the summary in listings.
- Sections in this order, as applicable: `# Examples`, `# Errors`, `# Panics`, `# Safety` (required on `unsafe fn`). Examples use `?` with the `fn main` hidden behind `# ` lines, never `unwrap()`, because users copy them verbatim. Describe parameters in prose, not a table.
- Document what the signature cannot say: preconditions, side effects, ownership transfer, complexity, and why. Do not restate the type or the name.
- Link items with ``[`Type`]``. Hide impls users never trigger (`From<PrivateError> for PublicError`) with `#[doc(hidden)]`; use `#[doc(inline)]` on `pub use` of crate-internal items.
- `//` comments explain why, not what: `// SAFETY:`, `// TODO(issue-123):`, non-obvious magic values, workarounds with a link. No design journals, "why we chose X" essays, or rule-compliance tables in code docs. Document the end state.

## Final checklist

- Format, clippy (`-D warnings`), and tests are clean.
- No `unwrap()` outside tests and `main`; every `expect`, `assert`, and `panic` has a message with the relevant values.
- Public functions take `&str`, `&[T]`, `impl AsRef<_>`, or `impl IntoIterator`, not `&String` or `&Vec<T>`.
- Public types derive `Debug` and the applicable std traits; error types implement `Error + Display + Send + Sync + 'static`.
- No `bool` parameters in public signatures; no `.clone()` whose only job is to satisfy the borrow checker.
- No globs (bar a framework prelude), no preludes of your own, no leaked third-party types.
- Every `unsafe` block has `// SAFETY:`; every `unsafe fn` has `# Safety` docs.
- Public items have a one-line summary and `# Errors`/`# Panics` where relevant.
- Application code stayed simple; library code followed the API rules.
