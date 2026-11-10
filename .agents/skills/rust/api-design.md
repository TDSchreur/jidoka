# API Design: Naming, Std Traits, Constructors, Visibility

Conventions for types and public APIs. Library code follows all of it; application code can relax the parts that only protect downstream users (semver, sealing, `#[non_exhaustive]`). Deeper treatment: [traits.md](traits.md), [type-design.md](type-design.md), [project-structure.md](project-structure.md), [references/standard-traits.md](references/standard-traits.md), [references/getter-setter.md](references/getter-setter.md), [references/bool-to-enum.md](references/bool-to-enum.md).

## Naming

| Item | Case | Notes |
|---|---|---|
| Types, traits, enum variants | `UpperCamelCase` | Acronyms are one word: `Uuid`, `HttpClient` |
| Functions, methods, modules, locals | `snake_case` | |
| Constants, statics | `SCREAMING_SNAKE_CASE` | |
| Type parameters | short, from the main bound | `T`, `R: Read`, `W: Write` |
| Lifetimes | short lowercase | descriptive (`'src`, `'conn`) when on a type |

- No `get_` prefix on getters: `len()`, `name() -> &str`, `name_mut()`. Bare `get` only when there is one obvious thing to get (`Cell::get`). Related: `set_foo(T)`, `take_foo()`, `with_foo(|f| ..)`, and unsafe `_unchecked` siblings of checked accessors.
- Conversion prefixes signal cost and ownership: `as_` (free, borrow to borrow), `to_` (expensive, to owned), `into_` (consumes `self`). Put `mut` in type order: `as_mut_slice`.
- Constructors: `new` (primary), `with_capacity` / `new_with_foo` (variants), `try_new` (fallible), `from_foo` (conversion; pair important ones with `From`). I/O resources use domain verbs: `File::open`, `TcpStream::connect`.
- Iterator methods `iter` / `iter_mut` / `into_iter` return `Iter` / `IterMut` / `IntoIter`; `keys()` returns `Keys`. If you implement `len()`, implement `is_empty()`.
- Error types read verb-object-error: `ParseBoolError`, not `BoolParseError`.
- No weasel suffixes `Service`, `Manager`, `Factory`, `Helper`, `Util`. Name the thing it is (`Bookings`, `Dispatcher`, `FooBuilder`); lifecycle belongs to `Drop`, repeatable instantiation takes `impl Fn() -> Foo`.
- Short names: at most two short words, no module name repeated in the type (`foo::Id`, not `foo::FooId`), no abbreviations an outsider would not know.
- Feature names are bare: `std`, `serde`, `alloc`, never `use-std` or `no-std`.

## Derive and implement the standard traits eagerly

Downstream crates cannot add std traits to your types (orphan rule). Derive whenever every field supports it:

- `Debug` on every public type (hand-write it to redact secrets and test that the secret never appears), `Clone` (not for unique resources or key material), `Copy` only for small bitwise-copyable types (the compiler copies silently), `PartialEq`/`Eq`, `PartialOrd`/`Ord` (never `PartialOrd` without `Ord` when a total order exists), `Hash` consistent with `Eq`, `Default`.
- `Display` for anything meant to be read: error types, string-like wrappers.
- `#[must_use]` on pure functions and on types whose discarding is a bug (errors, lazy values).
- Implement `From`/`TryFrom`, never `Into`/`TryInto`. Prefer `from`/`into` over lossy `as`. Implement `FromStr` instead of an ad-hoc `from_str`; `FromIterator`, `Extend`, and `IntoIterator` (owned and borrowed) on collections. `Deref` only on smart pointers and owning collections, never to fake inheritance. Operators only for their mathematical meaning.
- No derivable bounds on struct definitions: `#[derive(Clone)] struct Good<T>`, not `struct Bad<T: Clone>`. Bounds go on the `impl` blocks that need them.
- A hand-written `Ord` needs hand-written `PartialEq`, `PartialOrd`, and `Hash` that agree with it (same fields, same ignored fields). Say in the docs which fields equality ignores.

## Constructors, builders, defaults

- Provide `new()` even when `Default` exists, and make them agree. Bag-of-options structs with public fields always get `Default` so callers can write `..Default::default()`.
- Up to two optional parameters: inherent constructors (`new`, `with_a`, `with_b`). Beyond that, or when construction validates or has side effects, add `Foo::builder()` returning `FooBuilder`: setters named after fields (`timeout(d)`, not `set_timeout`), infallible, with `build() -> Result<Foo, Error>` doing all validation including cross-field checks. Required parameters go to the builder's constructor, not setters. No public `FooBuilder::new()`.
- Four or more loose constructor arguments group into semantic types: `Deposit::new(account, amount)`.
- Newtypes that encode an invariant validate in a fallible constructor (`TryFrom`, `FromStr`, `try_new`) and never offer an infallible `From` from the raw type.

## Visibility and evolution

- Private by default; widen to `pub(crate)` or `pub(super)` before `pub`. Making an item more public later is non-breaking, the reverse is breaking.
- Mark public enums and public-field structs that cross crate boundaries `#[non_exhaustive]` unless the set is a stable ontology. Match a foreign one with a wildcard arm.
- Seal traits you do not want implemented downstream (`pub trait T: private::Sealed`) and say so in the docs.
- Expose each public item at exactly one path. Do not `pub use` a foreign crate's items unless you are an umbrella crate, and do not glob re-export (`pub use foo::*`); re-export individually. Avoid leaking third-party types into the public API; feature-gate `serde` impls behind a feature named `serde`. Never define a `prelude`.
- Return `impl Iterator<Item = T>` (internal APIs) or a newtype (stable public APIs) instead of a `Vec` collected only to return.
- Mark a function `const` only if it will stay `const`. Do not expose `ErrorKind` enums or other internals that lock in failure modes you consider private.
- Keep `Rc`, `Arc`, `Box`, and `RefCell` out of public signatures; a service that must be shared implements a cheap `Clone` around an `Arc<Inner>`.
- Semver-breaking changes: removing or renaming public items, adding a variant to a non-`#[non_exhaustive]` enum, adding a field to a publicly constructible struct, adding a trait bound, making a trait non-object-safe. Deprecate in a minor release, remove in a major one.

## Traits and generics

- Define a trait only for generic programming (`T: Trait`, `impl Trait`, `dyn Trait`) or as an extension trait (`FooExt`), not to organize code. Essential functionality is inherent; trait impls forward to it so callers need no import.
- Prefer concrete types, then generics, then `dyn Trait` wrapped in your own struct (`struct DynStore(Arc<dyn Store>)`). Do not nest generics on service-level types (`Service<Backend>` is fine, `Service<Backend<Store>>` is not).
- Decide early whether a trait must be object-safe; exclude generic methods with `where Self: Sized`. Keep required methods minimal and give defaults to everything derivable from them.
- A function with a clear receiver is a method; a computation without one is a free function, not `Database::check(params)`.
- Closures: the most general bound that works (`FnOnce`, then `FnMut`, then `Fn`), preferred over `fn` pointers; closure parameters go last and at most one. Keep parameter order consistent across a crate: call-specific first, ubiquitous context (logger, allocator) last.
- Macros only when functions, generics, and traits cannot do the job; `macro_rules!` before proc macros; a macro never changes a type's kind or a function's signature. See [macros.md](macros.md).

## Refactoring someone else's code

- Fix every smell the request is about, not only the cheap ones: clones that only satisfy the borrow checker, `&String`/`&Vec<T>` parameters, `unwrap` on fallible paths, `Rc<RefCell<_>>` in signatures, `get_` getters, `bool` parameters, `+` string concatenation, `#![deny(warnings)]`, error types without `Error`, nondeterministic `HashMap` iteration in output.
- Keep public behavior identical unless changing it is the point: same operations, same success and failure semantics, same output bytes. Turning a panic into an `Err` is the change callers want; dropping an error variant, removing a derive, or changing what a constructor takes is not. If a signature must change, show before and after and say why.
- Preserve quirks you do not understand and flag them instead of silently fixing them.
- When the module has no tests, pin the current behavior with a few before touching it.
