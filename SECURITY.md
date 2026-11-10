# Security policy

## Supported versions

Only the [latest release](https://github.com/TDSchreur/jidoka/releases/latest) gets security fixes. Re-run the installer to update.

## Reporting a vulnerability

Report vulnerabilities privately through [GitHub's private vulnerability reporting](https://github.com/TDSchreur/jidoka/security/advisories/new). Do not open a public issue.

Include what is affected, how to reproduce it, and the impact you expect. You get a first answer within a week. Once a fix is released, the advisory is published with credit to you, unless you prefer otherwise.

## Security model

- **The dashboard is local.** It listens on localhost only and has no user accounts. Other users on the same machine can read and change runs, so use it only on a machine you trust. Writes need a per-start CLI token that is stored in a file only you can read.
- **Releases are verified.** The installers check every download against the SHA256 checksum published with the release.
- **No secrets are stored.** Run records and usage tracking keep references, short summaries and token counts. They never store prompts, conversation text, tool output or credentials.

For the details, see [Runs and the dashboard](docs/runs.md#security) and [Token usage](docs/usage-tracking.md#token-usage).
