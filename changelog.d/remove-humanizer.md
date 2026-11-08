## Removed

- The Humanizer skill is no longer bundled, and the workflows, the `pr` skill and the dashboard help no longer tell agents to run prose through it. `factory install` and `factory update` remove a `humanizer` skill folder that an earlier install created, in every skills folder including each `--config-dir`, but only when it is an unmodified copy of what the factory installed. A Humanizer you installed or edited yourself stays untouched.
