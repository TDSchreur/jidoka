## Removed

- A model-routing policy in the earlier format (roles and tasks, or the single-agent form) is no longer recognised or replaced by `jidoka install`, `jidoka doctor --create-model-routing` or the skills refresh. It is an invalid policy like any other, and an existing file is never overwritten.
- `jidoka install` no longer removes skill folders left by earlier versions, including the Humanizer skill.
- Run records written by earlier versions get no special handling: the `jidoka` mode is no longer read as `jidoka-full`, and a run already finished without a Finish summary no longer accepts further done updates.
