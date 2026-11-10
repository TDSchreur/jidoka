---
description: 'Keep documentation and changelog fragments in sync with code changes'
applyTo: 'src/**,dashboard/**,build.rs,Cargo.toml,scripts/**,skills/**,.agents/skills/**,.github/workflows/**'
---

# Update docs with code changes

Update documentation in the same pull request as the change.

- Add a fragment to `changelog.d/` for every user-visible change (commands, flags, dashboard behavior, installers, bundled skills). Follow the format of the existing fragments (`## Added`, `## Changed`, `## Fixed`).
- Update the guide in `docs/` that covers the change (`docs/installation.md`, `docs/runs.md`, `docs/pull-requests.md` and so on) when install steps, commands, options, or supported agents change, and `README.md` when the short install steps, basic usage or the list of guides change.
- Update `skills/README.md` when a skill is added, removed, renamed, or its upstream source or commit changes. Keep `build.rs` and the third-party license files consistent with it.
- Update `docs/releasing.md` when the release workflow, branch rules, or rulesets change.
- Update `GLOSSARY.md` when a domain term is introduced or its meaning changes, and `docs/specs/` when behavior that a spec describes changes.
- Update examples and command output in docs when behavior changes, and verify that linked files still exist.
- Keep internal refactors and test-only changes out of the changelog.
