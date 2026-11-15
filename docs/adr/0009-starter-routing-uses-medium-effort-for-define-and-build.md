---
status: accepted
---

# Starter routing uses medium effort for Define and Build

The starter model-routing policy used low effort for Define and Build. Across 28 Balanced and Full runs, 21 needed at least one Rework. Reasoning output is a small part of token use (Sonnet output was about 330k tokens in total), while a Rework repeats Build and Check and re-reads the context through the cache, which costs more. So Define and Build now default to medium effort for Claude Code, Codex and Copilot CLI. Check stays medium, Finish stays low, and the models do not change.

## Considered Options

- **Keep low effort.** Rejected: it saves little output and leads to Reworks.
- **High effort.** Rejected: more cost without evidence that it helps.
- **A stronger model such as opus for Build.** Rejected: cost, and the coordinator already runs opus.

## Consequences

- Existing `model-routing.json` files are not changed. To get the new defaults, edit the file or delete it and run `jidoka doctor --create-model-routing`.
- The evidence is observational, not an A/B comparison. Revisit this when the Rework rate changes.
