## Changed

- Errors that were silently dropped are now reported with their cause: a session log that cannot be read (usage and request notes), a pull request state or merge commit that `run cleanup` cannot read when offering the factory reinstall, the reason a worktree is refused as not on the recorded branch, a failed rollback when a skill folder is replaced, and background dashboard tasks that stop unexpectedly.
