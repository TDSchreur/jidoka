## Fixed

- `factory pr wait` no longer watches a stale head right after a push. GitHub can report the previous commit for a few seconds; when the run's worktree is on the pushed commit, the wait now gives the pull request up to 30 seconds to show it, instead of ending at once with `head-changed`.
- Azure Repos pull request URLs keep `-`, `.`, `_` and `~` as they are, so `discord-workshop` is no longer recorded as `discord%2Dworkshop`. Both forms open the same page, and `factory pr wait` reads both.
- The Azure Boards tracker instructions remove a triage role in a way that works. `az boards work-item update --fields System.Tags=...` only adds tags, and `show --fields` fails, so roles are now read with `--query` and written back with a JSON Patch `replace` through `az rest`.
