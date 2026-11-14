---
status: accepted
---

# Codex attention detection does not wait for subagents

Claude Code and Copilot attention detection accounts for background work, but a Codex coordinator can finish while a child is still running and need the user to resume it. In the session logs examined for [issue #448](https://github.com/TDSchreur/jidoka/issues/448), child-to-coordinator messages did not trigger a turn, including when the child finished after the coordinator. We will keep reporting a finished Codex turn as needing attention even when a child is outstanding, because suppressing that report could hide a real pause.

This is a deliberate difference in detection based on the available evidence, not a guarantee that Codex cannot wake a coordinator automatically. If future logs show a child triggering a finished coordinator, revisit the decision and use a reliable way to distinguish waking children from those that still need user intervention. The interaction between an eventual `request_user_input_async` answer and child completion also remains unverified.
