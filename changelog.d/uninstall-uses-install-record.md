## Changed

- `jidoka uninstall` uses the agents and Claude configuration directories recorded by `jidoka install` when `--agent` is omitted, removes the install record, and removes the model routing policy together with the run data. It deletes the binary (and the Windows PATH entry) only when the install script put it in place.
