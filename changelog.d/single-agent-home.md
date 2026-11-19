## Changed

- Jidoka now uses one home directory per agent for skills and session logs. Replace Claude's `--config-dir` option and `JIDOKA_CLAUDE_CONFIG_DIRS` list with the native `CLAUDE_CONFIG_DIR` environment variable. To use a different Claude profile, set that variable for the invocation. Skill refresh resolves the directory from the current environment.
