## Added

- Codex (OpenAI) models now have an estimated cost from the Standard-tier API list prices, with the long-context rates for requests above 272,000 input tokens. A row of an OpenAI model with cache writes, and models without a published price, stay `unknown`. The cost is a token-based estimate, not what a subscription is charged.

## Fixed

- Codex token usage is attributed more reliably: `jidoka run begin`, `run start` and `run update` record the Codex session from `CODEX_SESSION_ID`, a run id passed through a shell variable (`R=<id>; jidoka run update $R`, also across commands) counts, and a run command that only appears in tool output, a file the agent read or a message no longer claims a Codex session. Both the usage and the request readers claim Codex sessions the same way.
