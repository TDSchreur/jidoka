## Added

- `factory doctor --agent claude|codex --create-model-routing` writes a starter `model-routing.json` in the factory's config folder and then runs its checks. It never overwrites anything already at that path or writes through a symbolic link there. Without the option, doctor still changes nothing.

## Changed

- A relative `XDG_CONFIG_HOME` is now ignored when looking for `model-routing.json`, as the XDG specification says.
