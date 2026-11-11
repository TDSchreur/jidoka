## Changed

- `--agent` on `jidoka install` and `jidoka uninstall`, and `JIDOKA_AGENT` on `jidoka install`, accept `all` and menu numbers. An empty agent list gives a clear error.
- `jidoka uninstall` names agents that were not recorded when they are mixed with recorded ones.

## Fixed

- `jidoka uninstall --agent` with an unreadable install record removes only the named agents' skills and keeps the record, the binary and the run data.
- A `--config-dir` given through a symlink matches the recorded directory on uninstall.
- The skills refresh lock no longer needs hard links.
