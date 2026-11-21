## Changed

- The CLI and dashboard show the same local stage gate blockers and indicate when Finish awaits a live CI check.
- Runs record the gate evidence for each accepted stage exit, including distinct outcomes for Rework, merge reconciliation and overrides. Earlier evidence survives changes to ticket plans and later Reworks.
