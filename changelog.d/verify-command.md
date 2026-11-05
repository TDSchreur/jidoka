## Changed

- The `factory`, `factory-balanced` and `factory-fast` skills, and `implement-spec-balanced` and `implement-spec-fast`, find the project's own verification commands (from its agent instructions, contributing docs or build files, and its CI) and run all of them after each ticket and before finishing. A missing tool or a failing check now blocks the run instead of being skipped.
