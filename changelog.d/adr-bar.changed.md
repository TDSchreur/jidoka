## Changed

- The `jidoka-domain-modeling` skill offers an ADR whenever a decision was a real trade-off and is hard to reverse or surprising, instead of requiring all three conditions. It also says to write the ADR when the decision is made, and to use status `proposed` when the reason is unknown.
- The `jidoka-pr` skill adds an optional `**Decision:**` line for the ADR a change adds or follows, and asks to flag an unrecorded hard-to-reverse trade-off under Merge Danger.
