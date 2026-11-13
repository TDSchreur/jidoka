## Changed

- The dashboard no longer needs port 4311. It tries 4311, takes a free port when that is taken, and writes its port, pid and build to `dashboard.json` in the data folder, which it removes again when it stops. The CLI finds the dashboard through that file. This lets a Windows and a WSL2 dashboard run side by side. A `--port` that is taken is an error.
- Breaking: `jidoka dashboard --port <port>` no longer creates a separate dashboard with its own data. There is one dashboard per data folder, and a second `jidoka dashboard` reports that one as already running. The per-port `runs-<port>.json`, `session-token-<port>` and `plans-<port>` files are no longer used, and there is no migration; move their content to `runs.json`, `session-token` and `plans/` by hand if you need it.
