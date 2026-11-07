## Added

- One `model-routing.json` can now configure Claude Code and Codex independently. An `agents` object has an entry per agent, each with its own `roles`, `tasks` and an optional `enabled`. The original single-agent form (`agent`, `roles`, `tasks`) keeps working and is never rewritten, and a file that mixes the two forms, repeats a key, names an unknown agent or has an empty `agents` is refused with the file and the entry named.
- `factory run start "title" --agent claude|codex|copilot` names the agent a run is in. The run records only that agent's policy. With the `agents` form the option is required and the start stops with a message that names the configured agents; an agent without an entry, or a switched-off entry, is not routed, and the first prints a notice that says which entry to add. Existing single-agent policies and existing `factory run start` invocations behave as before. The shipped `factory-procedures` skill passes the option.
- `factory doctor` checks each configured agent's entry, tagged with the agent, or only the one named with `--agent`. An agent without an entry, a switched-off entry, and a single-agent file written for the other agent are reported as skipped.

## Changed

- `factory doctor --create-model-routing` writes the `agents` form with an entry for Claude Code and one for Codex. `--agent claude` or `--agent codex` writes an entry for that agent only. It still never overwrites an existing file.
