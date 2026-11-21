## Changed

- Cost estimates now price `claude-haiku-5-5` and `claude-fable-5-2`, and use the listed $0.10 cache read for `claude-sonnet-5-5` (it was $0.20). Haiku 5.5 is priced at its up-to-100K-token rates only, so requests above 100K input tokens are underestimated; see ADR-0022.
