---
status: accepted
---

# Attention excludes automatic context compaction

The dashboard derives `needs_you` in its run list and detail responses. A blocked run qualifies unless its current block is a recognized automatic context compaction. Compaction clears on later agent activity. Its blocked status, reason and events still explain the pause.

The stored reason and owner carry the evidence used by this rule.

| Block | Signal | Evidence in the run record |
| --- | --- | --- |
| Explicit request | A coordinator invokes Claude Code's `AskUserQuestion`, Codex's `request_user_input` or `request_user_input_async`, or Copilot's `ask_user`. | Pause source, time and “Agent stopped; it may need you”. The outstanding request is tracked during log collection. |
| Inferred wait | The coordinator finishes a response while the run is active. | The same pause source and reason as an explicit request. The user can check the session for a question or prompt it to continue. [ADR-0002](0002-codex-attention-and-subagents.md) still governs Codex turns with outstanding children. |
| Usage limit | Claude Code logs a rate-limit error. | Pause source, reason and pause time, with a resume time when activity returns. The user can check the reset time or resume when capacity returns. |
| Compaction | Claude Code logs a context-compaction boundary. | Pause source and reason, with the same timing evidence as a usage limit. |
| Agent block | The coordinator writes a blocking reason. | Agent source, reason and block event. |
| Workspace failure | Workspace setup fails and records a retryable problem. | Workspace source, reason and block event. |
| Pull request block | The dashboard observes a closed, unmerged pull request. | Pull request source, reason and block event. |

The source alone groups together compaction and requests for input. The reason alone could hide an agent-owned block that happens to use the same wording. For older runs that lack a source, the exact compaction reason already identifies an automatic pause in the lifecycle. Any unfamiliar reason stays visible under **Needs you**. The derived field gives every consumer the same rule without changing the stored format or introducing another block owner.
