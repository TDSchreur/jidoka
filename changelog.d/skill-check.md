## Changed

- When a ticket changes what an agent skill makes the agent do, Build agents now verify the change with a fresh agent (using the agent's skill-creator skill when it has one) and record a `Skill check:` line in the commit. Wording-only changes need none.
- The code review flags such a change when no `Skill check:` is recorded, and the pull request description carries it in Evidence.
