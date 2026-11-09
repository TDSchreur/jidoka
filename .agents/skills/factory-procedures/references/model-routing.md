# Model routing

The workflows read this at the start of each Stage they have, and again at the start of every Rework and every Check pass after it.

A run can carry a model-routing selection: the model and effort each Stage's agents run on. The policy file can describe several agents, and the run records only the entry of the agent named with `--agent` when it started, never another agent's. The coordinating session, which is you, keeps its own model: you talk to the user, report to the dashboard and run the `factory` commands. A routed Stage hands its work to fresh agents started on that Stage's model and effort.

## Reading the selection

Read it with `factory run inspect <id> --json`. `model_routing` holds `agent` (`claude`, `codex` or `copilot`), `stages` and `overrides`; both map a Stage (`define`, `build`, `check`, `finish`) to a `model` and an optional `effort`. The choice for a Stage is `overrides.<stage>`, otherwise `stages.<stage>`. The Stage is not routed, and keeps the session's model, when:

- the run has no `model_routing`, or neither map has the Stage;
- `agent` is not the agent you are running in;
- `factory doctor` warned that the policy cannot apply here.

Define and Finish are also not routed when the run's mode is `factory-fast`, or when the change is small: a clear, bounded request planned in the conversation, without a spec or tickets. Routing those two Stages costs more than it saves there, so they keep the session's model. Build and Check are routed as above.

## Starting a routed agent

- On Claude Code, the Agent tool's `model` accepts only the aliases `opus`, `sonnet`, `haiku` and `fable`, so pass the recorded model only if it is one of them. Set `effort` to the recorded effort, and start a fresh subagent, never a fork, because a fork ignores `model`.
- On Copilot CLI:
  - Start a fresh agent with the `task` tool, passing the recorded model as `model` and the recorded effort as `reasoning_effort`. The model must be an exact ID.
  - An unknown or unavailable model silently runs on the session's model, so when the agent finishes, check the model it ran on: `usage.model` of the `session.usage_record` events whose top-level `agentId` is that agent's (for example with jq: `jq -r 'select(.type=="session.usage_record" and .agentId=="<agent-id>") | .data.usage.model' events.jsonl`, or any JSON tool), in the session's `events.jsonl` under `$COPILOT_HOME/session-state/<session-id>/` (`~/.copilot/session-state/<session-id>/` when `COPILOT_HOME` is not set). The `model` in the agent's start and completed events is only what was requested.
  - Compare the models you read with the requested one before you write the Stage summary. Any record with a different model is a failed switch: keep the agent's work and report the switch as Reporting below says. With no record for the agent, you cannot confirm the model, so report it as not applied with that reason. When `factory-code-review` starts the review agents, it does this check and reports the result to you.
- On Codex, set the model and effort of the agent you start through the subagent settings Codex documents.
- If the recorded model is not one the host takes, or the host refuses a setting, such as a Claude Code that rejects `effort`, start the agent with neither the model nor the effort, so an effort is never paired with a model it was not chosen for.
- Give each agent what it needs to work on its own: the run's worktree, the spec or plan reference, its task and the skill to follow. For a ticket that changes an agent skill, also point it to "Changing agent skills" in the procedures. It reports back to you, and you record its result on the run.

## Per Stage

- **Define**: the interview (factory-grill-with-docs) stays with you, because it is a conversation with the user. Unless the run is `factory-fast` or the change is small, writing the spec, writing the tickets and fact-finding lookups in the code or docs run in agents on the define choice.
- **Build**: each ticket runs in its own fresh agent on the build choice, one at a time and sequentially in the run's worktree, following the workflow's build skill; so do a Rework's fixes. You still run the verification the workflow requires, or confirm the agent ran it on exactly this code, and you update the run's tickets and Stage.
- **Check**: tell `factory-code-review` the check choice, because its Standards and Spec sub-agents are the agents to route. Run `factory-security-review` on the check choice only when you start it as a separate agent the same way. Run inline in your own session, it uses your model: do not claim a switch for it.
- **Finish**: unless the run is `factory-fast` or the change is small, an agent on the finish choice writes the pull request description and fixes CI failures. You run `factory pr open`, `factory pr wait` and `factory pr sync` yourself.

When a Stage is not routed, do its work as the workflow says, without handing it to agents for routing's sake.

## Reporting

Say what happened in each Stage summary, with one of these:

- "Build agents ran on sonnet, effort low", when the Stage was routed and the agents ran as recorded.
- "model routing not applied: <reason>", when the Stage is not routed for one of the reasons above (for Define and Finish, "model routing not applied: factory-fast" or "model routing not applied: small change"), the model is not one the host takes, or the host refused a setting.
- "model routing not applied: Copilot ran <actual> instead of <requested>", when the check on Copilot CLI above finds a silent fallback.
