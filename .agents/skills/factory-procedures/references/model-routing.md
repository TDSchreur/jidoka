# Model routing

The workflows read this at the start of each Stage they have, and again at the start of every Rework and every Check pass after it.

A run can carry a model-routing selection: the model and effort each Stage's agents run on. The policy file can describe several agents, and the run records only the entry of the agent named with `--agent` when it started, never another agent's. The coordinating session, which is you, keeps its own model: you talk to the user, report to the dashboard and run the `factory` commands. A routed Stage hands its work to fresh agents started on that Stage's model and effort.

## Reading the selection

Read it with `factory run inspect <id> --json`. `model_routing` holds `agent` (`claude` or `codex`), `stages` and `overrides`; both map a Stage (`define`, `build`, `check`, `finish`) to a `model` and an optional `effort`. The choice for a Stage is `overrides.<stage>`, otherwise `stages.<stage>`. The Stage is not routed, and keeps the session's model, when:

- the run has no `model_routing`, or neither map has the Stage;
- `agent` is not the agent you are running in;
- you run in Copilot CLI, which records no selection and is never routed;
- `factory doctor` warned that the policy cannot apply here.

## Starting a routed agent

- On Claude Code, the Agent tool's `model` accepts only the aliases `opus`, `sonnet`, `haiku` and `fable`, so pass the recorded model only if it is one of them. Set `effort` to the recorded effort, and start a fresh subagent, never a fork, because a fork ignores `model`.
- On Codex, set the model and effort of the agent you start through the subagent settings Codex documents.
- If the recorded model is not one the host takes, or the host refuses a setting, such as a Claude Code that rejects `effort`, start the agent with neither the model nor the effort, so an effort is never paired with a model it was not chosen for.
- Give each agent what it needs to work on its own: the run's worktree, the spec or plan reference, its task and the skill to follow. It reports back to you, and you record its result on the run.

## Per Stage

- **Define**: the interview (grilling) stays with you, because it is a conversation with the user. Writing the spec, writing the tickets and fact-finding lookups in the code or docs run in agents on the define choice. A workflow that plans in the conversation hands the writing of its plan to such an agent.
- **Build**: each ticket runs in its own fresh agent on the build choice, one at a time and sequentially in the run's worktree, following the workflow's build skill; so do a Rework's fixes. You still run the verification the workflow requires, or confirm the agent ran it on exactly this code, and you update the run's tickets and Stage.
- **Check**: tell `code-review` the check choice, because its Standards and Spec sub-agents are the agents to route. Run `security-review` on the check choice only when you start it as a separate agent the same way. Run inline in your own session, it uses your model: do not claim a switch for it.
- **Finish**: an agent on the finish choice writes the pull request description and fixes CI failures. You run `factory pr open`, `factory pr wait` and `factory pr sync` yourself.

When a Stage is not routed, do its work as the workflow says, without handing it to agents for routing's sake.

## Reporting

Say what happened in each Stage summary: "Build agents ran on sonnet, effort low", or "model routing not applied: <reason>" when the Stage is not routed for one of the reasons above, the model is not one the host takes, or the host refused a setting.
