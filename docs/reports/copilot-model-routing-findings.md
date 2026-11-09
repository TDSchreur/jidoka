# Copilot CLI model routing findings

How a main agent in Copilot CLI picks the model and reasoning effort of a subagent, and how to see what actually ran. Checked against Copilot CLI 1.0.107 (`copilot help config`, `copilot --help`, the GitHub Docs custom agents reference) and by running sessions on 2026-11-09.

## Ways to choose a subagent's model

1. **`task` tool parameters.** `model` and `reasoning_effort` are optional. The `model` parameter is advertised as an enum of model IDs, and `reasoning_effort` as a free string. An explicit `model` wins over a custom agent's own `model`.
2. **Custom agent `model` frontmatter.** An `*.agent.md` file (for example `.github/agents/`) may set `model: <id>`. The GitHub Docs describe it as "Model to use when this custom agent executes. If unset, inherits the default model." There is no `reasoning_effort` frontmatter field in that reference.
3. **User configuration.** `subagents.agents.<agent-name>` in the config holds `model`, `effortLevel` and `contextTier` for each subagent. Each can be `"inherit"`. `/subagents` edits it. This is the user's setting, not something the main agent can pass.

With no setting at all, a subagent uses the session's model and effort.

## Accepted values

Model IDs listed by `copilot help config` for 1.0.107 (the available set depends on the account and plan):

- Claude: `claude-sonnet-5.5`, `claude-sonnet-5`, `claude-fable-5.1`, `claude-fable-5`, `claude-opus-5.5`, `claude-opus-5`, `claude-opus-4.8`, `claude-opus-4.8-fast`, `claude-sonnet-4.6`, `claude-haiku-5.5`, `claude-haiku-4.5`
- GPT: `gpt-6.1-sol`, `gpt-6.1-terra`, `gpt-6-sol`, `gpt-6-luna`, `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.3-codex`
- Others: `mai-code-1.1-flash`, `gemini-4-argon`, `gemini-3.8-flash`, `grok-4.7`, `grok-4.6`, `kimi-k3`
- `auto` is accepted by `--model` and `/model`. It was not tried as a `task` model.

Reasoning effort levels (`--reasoning-effort`): `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`. "Supported levels depend on the model." The `task` tool's own description says `xhigh` and `max` are available only for some models.

Names are exact IDs with dots, for example `claude-haiku-4.5`, not aliases such as `haiku`. The Claude Code aliases (`opus`, `sonnet`, `haiku`) were not tried here.

## What happens with bad values

All results below come from the session's `events.jsonl`. In every case the call succeeded and the subagent replied. Nothing was rejected and no warning was shown.

| Request | Result |
| --- | --- |
| `claude-haiku-4.5`, effort `low` | Ran on `claude-haiku-4.5`. |
| Unknown model `not-a-real-model` | **Silently ran on the parent's model** (`claude-sonnet-5.5`). The tool schema enum was not enforced. |
| Valid model, unknown effort `ultra` | Ran on the requested model. The effort was recorded as `ultra` in the events but is ignored; the usage record has no effort for it. |
| `claude-sonnet-4.6` (listed in the config help) | **Silently ran on the parent's model.** It is probably not available to this account; the CLI does not say so. |
| `gpt-5.6-terra`, effort `high` | Ran on `gpt-5.6-terra`, effort `high`. |
| `claude-opus-4.8`, effort `xhigh` | Ran on `claude-opus-4.8`, effort `xhigh`. |
| Custom agent with `model: not-a-real-model` | Ran on the parent's model. `modelSelectionSource` is `agent_definition_default`, so the fallback is not marked. |
| Custom agent with `model: claude-haiku-4.5` | Ran on `claude-haiku-4.5`. |
| Custom agent with `model: claude-haiku-4.5`, plus `task` `model: claude-sonnet-4.6` | The explicit `task` model took precedence, then fell back to the parent's model as above. |

Takeaway: a name appearing in the docs or config help does not mean it ran. Always check the events, because a wrong or unavailable model degrades silently to the parent's model, which is also the more expensive failure when the parent is the larger model.

An observation not explained: the fallback agents recorded `reasoningEffort` `low` in one session and `medium` in another. Do not rely on the effort of a fallback.

## Observing the model that ran

Each session writes `~/.copilot/session-state/<session-id>/events.jsonl`, one JSON event per line. Subagent events carry `agentId`.

- `subagent.started`: `model`, `taskModelSource` (`task_argument` or `custom_agent_definition`), `modelSelectionSource` (`explicit_override` or `agent_definition_default`). These record the **requested** model, even when it is unknown.
- `subagent.configured`: requested `model` and `reasoningEffort`, again as requested.
- `subagent.completed`: `model` (requested) and **`firstDispatchedModel`**, the model the first call went to. A difference between the two means the request was not honoured. For the unknown model, `model` was `not-a-real-model` and `firstDispatchedModel` was `claude-sonnet-5.5`.
- `session.usage_record` with the subagent's `agentId`: `usage.model` and `usage.responseModel` are what the API served, and `usage.cost` is the billing multiplier (haiku `0.33`, sonnet `1.0`, opus `15.0` in these runs). `usage.reasoningEffort` is present when the model supports effort.
- `session.model_change` with `source: "agent"`: repeats the requested model and effort.

Reliable check: compare `usage.model` of the subagent's `session.usage_record` with the model you asked for. Do not trust `subagent.started`, `subagent.configured`, or the `model` field of `subagent.completed`.

```bash
python3 - <<'PY'
import json, sys
for line in open(sys.argv[1]):
    e = json.loads(line)
    d = e["data"]
    if e["type"] == "subagent.completed":
        print("asked", d["model"], "first dispatched", d["firstDispatchedModel"])
    elif e["type"] == "session.usage_record" and e.get("agentId"):
        print("served", d["usage"]["model"], d["usage"].get("reasoningEffort"), d["usage"]["cost"])
PY
```

Pass the `events.jsonl` path as the argument.

## Limits and what was not tried

- One account and plan. Availability of listed models differs by account.
- The subagent's reasoning effort is only visible in `usage.reasoningEffort`, and only for models that support effort.
- Not tried: `auto` as a `task` model, `max` effort, `context_tier`, `/subagents` and `subagents.agents.*` configuration, and a custom agent's `model` combined with an effort.

## Implications for the factory

The factory records no model routing for Copilot CLI (`factory run begin --agent copilot` prints "Model routing is not supported on Copilot CLI"). If that is revisited, validate every routed stage afterwards against `usage_record.usage.model`, never against the requested name, and treat a mismatch as a failed switch.
