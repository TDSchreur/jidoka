# Show factory runs on the dashboard

The dashboard should show progress when an agent uses factory. The local Rust server owns durable run records and provides them to the dashboard. The factory skill registers a run and updates its stage as work progresses; completed and blocked work remain visible. The dashboard refreshes automatically and clearly reports when the server is unavailable.

Keep the system local and small: a run needs a title, project, stage, and status. The CLI talks to the server, so the skill does not manipulate stored data directly. Starting the dashboard starts the server. Existing runs survive a server restart. Do not require a database or external service.

Verify run creation and updates, persistence across restart, invalid requests, and dashboard display. Preserve the existing CLI help and version commands.
