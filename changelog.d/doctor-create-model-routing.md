## Added

- `factory doctor --agent claude|codex --create-model-routing` writes a starter `model-routing.json` in the factory's config folder and then runs its checks. It never overwrites or follows anything already at that path. Without it, doctor still changes nothing.
