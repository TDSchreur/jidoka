## Added

- `factory dashboard` finds a dashboard that is already running through the new `/api/info` route (pid and Factory version), prints its URL and exits instead of failing to bind. It warns when the running build differs and never stops it.
- The printed URL ends in `?project=<path>`, so the page shows that Project's row first and expanded.
- `factory dashboard --port <port>` runs a separate dashboard on another port with its own `runs-<port>.json`.
