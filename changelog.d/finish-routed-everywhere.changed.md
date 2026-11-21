## Changed

- Finish is now routed in `jidoka-fast` runs and small changes too, so the pull request text uses the Finish model; Define still stays on the coordinator there. See [ADR-0018](../docs/adr/0018-finish-is-routed-in-every-mode-and-starters-offer-complex-models.md).
- The starter model-routing policy reviews on `gpt-6.2-astra` for Copilot CLI (it was `claude-sonnet-5.5`) and gives every agent a `complex` map with a stronger model at high effort for Define and Build. `jidoka install` replaces your policy with these defaults.
- The model routing docs no longer link to a missing findings report and tell you to run `jidoka doctor` after editing the policy.
