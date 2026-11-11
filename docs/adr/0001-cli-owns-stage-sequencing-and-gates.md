---
status: accepted
---

# The CLI owns stage sequencing and gates

Stage order and exit Gates lived in skill prose, so every agent read several kilobytes of instructions up front and nothing checked that a Gate had passed. We decided that the `jidoka` CLI owns both: it holds the Stage graph per Workflow mode as data, tells the agent what to do next (`jidoka run next`), records Gate results (`jidoka run gate`) and is the only way to change Stage (`jidoka run advance`). Skills shrink to a short bootstrap, and `run next` serves only the current Stage's instructions.

## Considered Options

- **Skill prose (status quo).** Simple to write, but the agent loads all instructions for the whole run, and a Gate is only a request the agent can skip or forget. Rejected: no enforcement and a high token cost.
- **A sub-agent orchestrator.** A manager agent starts a fresh agent per Stage and checks the results. Rejected: it is not equally available in Claude, Codex and Copilot (the parity rules), it adds a layer to build and debug, and a manager's judgement is no more reliable than a deterministic check. It stays out of scope; the CLI approach does not rule it out later.

## Consequences

- Breaking change, accepted on 0.x: `jidoka run update` no longer changes the Stage, the run storage format changes, and runs in flight at the upgrade are not migrated.
- Gates that need judgement (such as a review) are recorded by the agent; the CLI checks what it can (tickets done, the head commit that verification passed on, pull request and CI state).
- One escape hatch, `run advance --force --reason "<text>"`, records the reason on the run for a stuck run.
- GitHub and Azure DevOps, and Claude, Codex and Copilot, see the same output. A Gate or Stage instruction that cannot be implemented for one of them needs its own ADR first.
