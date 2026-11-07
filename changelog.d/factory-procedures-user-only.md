## Changed

- `factory-procedures` can no longer be started by the model on its own: it sets `disable-model-invocation: true`, and for Codex `allow_implicit_invocation: false`. The workflows still read it through their links.
- The factory workflows name the user-only skills they use (`grill-with-docs`, `to-spec`, `to-tickets`, and in `/factory-full` also `implement-spec`) and say to read their `SKILL.md` instead of invoking them, because an agent's skill tool refuses skills that only the user may invoke.
