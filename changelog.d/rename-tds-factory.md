## Changed

- Rename Factory 2 to TDS Factory: the CLI is now `factory`, the workflow skills are `/factory` and `/factory-fast`, the installer variable is `FACTORY_AGENT`, and runs are stored under `tds-factory/`. Runs recorded under `tds-factory2/` are not migrated.
- `factory install` removes the old `factory2` and `factory2-fast` skill directories. Delete the old `factory2` binary manually.
