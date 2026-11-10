## Added

- Codex (OpenAI) models now have an estimated cost from the Standard-tier API list prices, with the long-context rates for requests above 272,000 input tokens. A row of an OpenAI model with cache writes, and models without a published price, stay `unknown`. The cost is a token-based estimate, not what a subscription is charged.
- Usage stored before this version has no long-context split, so its requests over 272,000 input tokens are priced at the short-context rate until the run's usage is collected again (on its next `jidoka run update`).

## Fixed

- Codex token usage is attributed more reliably: `jidoka run begin` and `run start` record the first valid session id of `CLAUDE_CODE_SESSION_ID`, `CODEX_SESSION_ID` and `CODEX_THREAD_ID`, and `run update` records every valid one, a run id passed through a shell variable (`R=<id>; jidoka run update $R`, also across commands) counts, and a run command that only appears in tool output, a file the agent read or a message no longer claims a Codex session. Both the usage and the request readers claim Codex sessions the same way.
