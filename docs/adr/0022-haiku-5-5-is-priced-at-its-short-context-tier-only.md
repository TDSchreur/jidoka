---
status: accepted
---

# Haiku 5.5 is priced at its short-context tier only

Claude Haiku 5.5 costs $0.10 input and $0.50 output per million tokens for prompts up to 100K tokens, and $0.50 and $2.50 beyond. Its reference lists no cache prices, so Jidoka uses the documented multipliers: 0.1x input for a cache read and 1.25x input for a 5-minute cache write.

The long-context split of a usage row uses one threshold for every model (`LONG_CONTEXT_INPUT_TOKENS`, 272K) and is persisted in the run record. Haiku's 100K boundary cannot be applied without changing that format. Jidoka therefore prices Haiku 5.5 at its up-to-100K tier only (no long-context rates), and requests above 100K input tokens are underestimated.

## Considered Options

- **A per-model threshold in the run record.** Rejected: it changes the run record format and the collected data for one model's tier, and old runs would need a migration or be split differently.
- **Price at the above-100K tier.** Rejected: it overestimates every normal request by five times.
- **Leave Haiku unpriced.** Rejected: its cost then shows as unknown and makes every total that includes it partial.

## Consequences

- Estimates for Haiku requests above 100K input tokens are too low. Cost is already an estimate at list prices, and Haiku is a small share of usage.
- If the split gains a per-model threshold later, Haiku's long-context rates can be added to the price table.
