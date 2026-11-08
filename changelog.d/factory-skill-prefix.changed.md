## Changed

- Every bundled skill now has a `factory-` name, so factory skills no longer collide with skills of other packs such as mattpocock/skills. Use `/factory-setup` (was `/setup-tds-factory`), `/factory-pr`, `/factory-tdd`, `/factory-triage` and so on:
  - `grill-with-docs`, `grilling`, `domain-modeling`, `codebase-design`, `to-spec`, `to-tickets`, `tdd`, `code-review`, `security-review`, `pr` and `triage` get a `factory-` prefix.
  - `implement-spec` is now `factory-implement`, `implement-spec-fast` is `factory-implement-fast`, `implement-spec-balanced` is `factory-implement-balanced`, and `setup-tds-factory` is `factory-setup`.
- `factory install` and `factory update` remove the folders with the old names. That includes same-named skills from other packs (for example mattpocock/skills), which you must reinstall afterwards.
