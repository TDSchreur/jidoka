## Changed

- The `setup-matt-pocock-skills` skill is now `setup-tds-factory`; run `/setup-tds-factory` once in each repository. `factory install` removes the old skill folder.
- GitLab is no longer a supported issue tracker. Setup offers GitHub, Azure Boards and local Markdown files in `.scratch/`.
- `factory run begin` adds `.factory/` and `.scratch/` to the new worktree's `.gitignore` when they are missing.
