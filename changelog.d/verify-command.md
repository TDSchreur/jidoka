## Changed

- The `factory`, `factory-balanced` and `factory-fast` skills, and `implement-spec-balanced` and `implement-spec-fast`, run the project's `scripts/verify.sh` when it exists, after each ticket and before finishing. A missing tool or a failing check now blocks the run instead of being skipped.
