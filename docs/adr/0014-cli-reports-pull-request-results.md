---
status: accepted
---

# CLI reports pull request results to the dashboard

The CLI reads pull request state from the host and posts the result to the dashboard so the cache and run transition are saved together. The dashboard trusts the authenticated CLI's URL, state and read-start timestamp. Missing runs return 404, superseded results return 409, and accepted results return a boolean indicating whether the run transitioned.

## Considered options

The dashboard could recheck the host instead of trusting the CLI, but that duplicates the request and could produce a different result. The CLI token already authorizes run updates.

## Consequences

Results for the same URL are ordered by read-start milliseconds. On a tie, merged wins over closed, which wins over other states; conflicting states of equal rank retain the first result. Repeated identical observations are accepted so they can reconcile a later run transition. A recorded merge is terminal against conflicting states. These rules make ties deterministic but cannot order conflicting nonterminal reads started in the same millisecond.
