## Removed

- The Humanizer skill is no longer bundled, and the workflows, the `pr` skill and the dashboard help no longer tell agents to run prose through it. `factory install` and `factory update` remove a `humanizer` skill folder that an earlier install created, recognised by its `LICENSE` matching the one the factory shipped, in every skills folder including each `--config-dir`. A Humanizer you installed yourself stays untouched.
