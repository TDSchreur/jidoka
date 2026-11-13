# Model routing

Model routing lets you say which model and effort each Stage runs on, independently of the workflow mode. Jidoka records your choices on the run. Every Stage can be routed: the coordinating session keeps its own model and talks to you, and each routed Stage's work is handed to fresh agents started on that Stage's model and effort. Without a policy file, runs behave as before.

Write one policy file, `model-routing.json`, in Jidoka's config folder: `$XDG_CONFIG_HOME/jidoka/` (or `~/.config/jidoka/` when unset) on Linux, and `%APPDATA%\jidoka\` on Windows. A relative `XDG_CONFIG_HOME` is ignored. One file can hold independent settings for Claude Code and Codex, each under its agent's name in `agents`:

```json
{
  "enabled": true,
  "agents": {
    "claude": {
      "stages": {
        "define": { "model": "sonnet", "effort": "low" },
        "build": { "model": "sonnet", "effort": "low" },
        "check": { "model": "sonnet", "effort": "medium" },
        "finish": { "model": "haiku", "effort": "low" }
      }
    },
    "codex": {
      "stages": {
        "define": { "model": "gpt-6.1-terra", "effort": "low" },
        "build": { "model": "gpt-6.1-terra", "effort": "low" },
        "check": { "model": "gpt-6.1-terra", "effort": "medium" },
        "finish": { "model": "gpt-6-luna", "effort": "low" }
      }
    },
    "copilot": {
      "stages": {
        "define": { "model": "claude-sonnet-5.5", "effort": "low" },
        "build": { "model": "claude-sonnet-5.5", "effort": "low" },
        "check": { "model": "claude-sonnet-5.5", "effort": "medium" },
        "finish": { "model": "claude-haiku-4.5", "effort": "low" }
      }
    }
  }
}
```

`jidoka install` creates this policy when no file exists, printing the path. It preserves any existing file, including an invalid policy and a symbolic link. `jidoka doctor --create-model-routing` can also create it explicitly, with all three agents or just the one named by `--agent`; see [Checking readiness](doctor.md).

- Each entry under `agents` is `claude`, `codex` or `copilot` and has `stages`, and optionally `enabled`. `stages` maps `define`, `build`, `check` and `finish` to a `model` and an optional `effort`; a Stage you leave out is not routed and keeps the session's model. `enabled` defaults to `true`; set it to `false` at the top to switch routing off for every agent, or inside an entry to switch off that agent only, without deleting anything. The defaults above favour cheap models and low effort to keep token use down.
- Models and efforts are plain text of up to 64 letters, digits and `. _ : / -`. Jidoka keeps no list of valid models, so a new model works without a Jidoka release, and checking that your agent accepts a value is up to you. Copilot CLI entries are checked a little further, because it quietly ignores what it does not know: the model must not be a Claude Code alias (`opus`, `sonnet`, `haiku`, `fable`; Copilot needs exact IDs such as `claude-sonnet-5.5`), and the effort must be one of `none`, `minimal`, `low`, `medium`, `high`, `xhigh` or `max`.
- `jidoka run start "title" --agent <claude|codex|copilot>` names the agent a run is in; the workflow skills pass it for you. Jidoka reads the file once, records only that agent's policy on the run, and never uses another agent's:
  - The option is required, because the file cannot tell which agent is asking. Without it the start stops and names the agents the file configures. An agent with no entry, or a switched-off entry, is not routed; the first also prints a notice that says which entry to add.
  - `--agent copilot` records the `agents.copilot` entry like the others, and is not routed when the file has none.
  - Continuing a run keeps the recorded selection, so editing the file affects only runs you start afterwards.
- A file that cannot be read, is too large, is not valid JSON, has an unknown field, Stage or agent, repeats a key (an agent or a Stage), has an empty `agents`, or has a bad model or effort stops `jidoka run start` with the file path and the problem, naming the entry (`agents.codex`) when the mistake is in one. Every enabled entry is checked each time, so a typo in Codex's entry also stops a Claude Code or Copilot run until it is fixed. A policy with `enabled: false`, and a switched-off entry, are not checked.
- To give one Stage a different model on a running run, use `jidoka run update <id> <status> --model-routing check=sonnet:medium` (the form is `<stage>=<model>[:<effort>]`; repeat the option for more Stages, and leave out `:<effort>` to set the model only). The options you give replace all earlier overrides, and `--clear-model-routing` removes them. The Stage must be `define`, `build`, `check` or `finish`, the model and effort follow the text rules above, and a run started without a policy has nothing to override. An override wins over the policy for that Stage only; the policy's own entries are not changed, and the override is kept when the run is continued.
- When a run's selection routes a Stage, the work of that Stage runs on the Stage's model and effort:
  - Define: the spec, the tickets and fact-finding agents. The interview stays with you, in the coordinating session. `jidoka-fast` runs and small changes (a clear, bounded request planned in the conversation, without a spec or tickets) keep Define on the session's model.
  - Build: each ticket, and each Rework, in a fresh agent.
  - Check: the review agents of `jidoka-code-review`, and `jidoka-security-review` when it runs as a separate agent; run inside the coordinating session it keeps the session's model.
  - Finish: the pull request description and CI fixes. The coordinator runs the `jidoka pr` commands. `jidoka-fast` runs and small changes keep Finish on the session's model.
  - On Claude Code the agents are fresh subagents started with the Agent tool's `model` and `effort`, which accepts only the aliases `opus`, `sonnet`, `haiku` and `fable`. On Codex they use its subagent model and effort settings. On Copilot CLI they are subagents started with the `task` tool's `model` and `reasoning_effort`, which take exact model IDs; an unknown or unavailable model silently runs on the session's model, so compare the models `jidoka run inspect` shows with what you asked for.
  - Each Stage summary says which model and effort its agents ran on, or why routing did not apply.
  - Without a selection, or for a Stage the selection leaves out, the Stage works as before. `jidoka-fast` has no Check, so its policy's `check` entry is ignored.
- `jidoka run next <id>` prints the current Stage's routed model and effort (the run's overrides first, then the policy file), or none when routing is off or leaves the Stage out. Only a routed Stage also gets the Model routing instructions for the run's agent, which say to hand the Stage's work to fresh agents on that model; `--json` carries the same as data.
- `jidoka run inspect <id>` shows each Stage's model and effort, or "not routed", under "Model routing", marks overridden Stages, and `--json` carries the selection in `model_routing`. Under each routed Stage it also lists the models the run's [token usage](usage-tracking.md#token-usage) shows for that Stage, with the reasoning effort the logs recorded and `+subagents` where a subagent used that model:
  - An alias such as `sonnet` and an observed id such as `claude-sonnet-5-5` differ in form, so both are shown. Only subagents are routed, so the request is judged by the models subagents used in that Stage: it is met when one of them matches, even if the coordinator ran another model, and a coordinator on the requested model does not satisfy it when the subagents ran a different one.
  - When subagents ran and the requested model never appears among them, the line says `MISMATCH`. When usage is only partial it says the model was not observed, because it may be missing, and every observed line notes partial coverage.
  - A Stage with only coordinator usage is neutral: it says `kept on the coordinator`, or `matches the request (kept on the coordinator)` when the coordinator's model matches, and never reports a `MISMATCH`, for the model or the effort.
  - A Stage with no usage rows, or a run whose usage is not tracked, says so.
  - When an effort was requested, it is compared with the efforts recorded for the subagent models that match the request. A different effort is a `MISMATCH` (or "not observed" when usage is partial), and a log that records none says `effort not recorded`. Claude Code, Codex and Copilot record the effort of each call, so the comparison works the same for a Copilot run (against exact IDs such as `claude-sonnet-5.5`). A Copilot call whose model does not support effort records none, and then says `effort not recorded`.
  - The request counts as met when any one of the recorded efforts is the requested one. When the coordinator and the agents run the same model at different efforts, their efforts land in the same row, so the agents' effort cannot be told apart from the coordinator's. A row keeps at most eight distinct efforts.
  - Runs without a selection, or recorded in the earlier shape, show no Model routing section.

Copilot CLI is routed through the `task` tool's `model` and `reasoning_effort` options. Which models an account can use differs, and an unavailable one silently falls back to the session's model, so check the observed models in `jidoka run inspect`. See [the findings](reports/copilot-model-routing-findings.md).

After a routed Stage finishes, usage collection records a non-blocking activity report for each observed model whose subagent usage differs from its assigned model. Coordinator-only usage never warns, because the coordinating session keeps its own model. `jidoka run inspect` shows these warnings and `--json` includes them in `run_reports`. Missing or disabled usage produces no warning.
