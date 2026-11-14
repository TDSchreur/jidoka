---
status: accepted
---

# The CLI finds the dashboard through a discovery file, not a fixed port

The dashboard used to own port 4311, and the CLI always talked to it. When 4311 was taken the dashboard could not start, and a Windows dashboard and a WSL2 dashboard, which share `localhost`, could not run side by side ([PR #438](https://github.com/TDSchreur/jidoka/pull/438)). We decided that the dashboard binds `--port`, else 4311, else a free port, and once it is listening writes `dashboard.json` (port, pid and build) in the data folder. The CLI reads that file, probes `/api/info` on the recorded port, and counts a dashboard as running only when the answering pid matches the file. A clean shutdown removes the file if it still names that pid.

## Considered Options

- **A fixed port with per-port data (the previous design).** Each port had its own `runs-<port>.json`, `session-token-<port>` and `plans-<port>/`, and the CLI assumed 4311. Rejected: a taken port blocked the dashboard, and the per-port names existed only to support that model.

## Consequences

- There is one dashboard per data folder. The per-port file and folder names are gone and are not migrated; data left in the old names is no longer read.
- A dashboard from an older build writes no `dashboard.json`, so the new CLI does not find it. Restart the dashboard after upgrading.
- The file has no lock. Two dashboards that reach liftoff at the same instant can both pass the "already running" re-check, and a port that is free when chosen can be taken before it is bound. Both were reviewed and left as they are.
- The CLI token file ([ADR-0004](0004-dashboard-posts-need-a-separate-cli-token.md)) lives in the same data folder and no longer depends on the port.
