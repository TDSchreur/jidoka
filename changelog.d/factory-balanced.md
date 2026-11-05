## Added

- `/factory-balanced` workflow mode between `/factory` and `/factory-fast`: lightweight Define with explicit acceptance criteria, implementation followed by behavior tests with the new `implement-spec-balanced` skill, one code review, and a security review only for security-sensitive diffs. `factory install` installs both new skills.

## Fixed

- `factory install` no longer adds a third-party `LICENSE` to the local `implement-spec-fast` skill.
