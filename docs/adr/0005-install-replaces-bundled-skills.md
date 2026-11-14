---
status: accepted
---

# Install replaces the bundled skills and overwrites local edits

`install` used to skip any skill directory that already existed, so an installed skill never received the shipped version. We decided that every install makes each bundled skill directory match the bundle exactly ([PR #20](https://github.com/TDSchreur/jidoka/pull/20)): files are written to a staging directory, swapped into place, and the previous directory is restored if the swap fails. Files dropped upstream are removed. Skills the CLI does not bundle are left alone.

## Considered Options

- **Skip existing skills (the original behaviour).** It protected local edits, but a skill directory that exists is never updated. Rejected in favour of always installing the latest bundled skills.

## Consequences

- Local edits to a bundled skill are lost on the next install. The README and installation guide say so. A skill that should survive installs needs a name the CLI does not bundle.
- Skills are refreshed automatically after the binary is upgraded: the first command run afterwards replaces them for the recorded agents without asking ([PR #373](https://github.com/TDSchreur/jidoka/pull/373)), so edits can disappear without the user running `jidoka install`.
