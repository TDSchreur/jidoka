## Added

- The usage reader keeps the reasoning effort that Claude Code and Codex record for each call. Each stored usage row has an `efforts` list, left out when a log records none. `factory run inspect` shows the effort next to each observed model and compares it with the effort requested for a routed task.
