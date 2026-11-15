## Changed

- The starter model-routing policy now uses medium effort for Define and Build (it was low) for Claude Code, Codex and Copilot CLI; see [ADR-0009](../docs/adr/0009-starter-routing-uses-medium-effort-for-define-and-build.md). Existing `model-routing.json` files are kept, so to get the new defaults edit the file or delete it and run `jidoka doctor --create-model-routing`.
