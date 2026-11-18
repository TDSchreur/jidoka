## Added

- `jidoka knowledge` stores a per-user knowledge repo checkout and your machine-local repository checkouts (`knowledge.json` in the config folder). `jidoka group create --spec` turns a file of the knowledge repo into its GitHub or Azure DevOps web link and refuses a file that is not pushed. When a knowledge repo is set, `jidoka run next` tells agents to read it in Define and record learnings in Finish.
- `jidoka group create` now also prints the spec it stored.
- Breaking (0.x): the interim `knowledge.md` that the knowledge repo skills wrote is no longer used. Run `jidoka knowledge set <path>` and `jidoka knowledge checkout <name> <path>` once; nothing is migrated.
