## Fixed

- `jidoka run stats` and the dashboard no longer say a run's session id was never recorded when its recorded session logs are in a folder that was not scanned. Like `jidoka run requests`, they name the sessions and the scanned folders and point to `CLAUDE_CONFIG_DIR`.
