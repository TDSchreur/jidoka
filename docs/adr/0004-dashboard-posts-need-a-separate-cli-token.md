---
status: accepted
---

# Dashboard POSTs need a separate CLI token

`POST /api/runs` and `POST /api/runs/<id>` accepted any request with a local Host, no Origin and a JSON content type, so any local process or other local user could create or change runs ([issue #300](https://github.com/TDSchreur/jidoka/issues/300)). Because the Reader serves a run's recorded Markdown from its repository, that also let a caller read `.md` files from any git repository the dashboard's user can read. We decided that POSTs need a **CLI token**, a second random token per dashboard start (32 bytes, 64 lowercase hex characters) sent in `X-Factory-Token`. The dashboard writes it to a file only the user can read, after it has bound its port, and the CLI reads that file on every POST ([PR #305](https://github.com/TDSchreur/jidoka/pull/305)).

## Considered Options

- **Reuse the page's session token.** Rejected: the page embeds it, so any local process can read it with `GET /`, and it would not close the hole. The session token keeps guarding only DELETE.

## Consequences

- On Linux and macOS the file is mode 0600 in a 0700 folder and is written atomically. On Windows it relies on the per-user ACL of `%LOCALAPPDATA%`; there is no explicit ACL and no new dependency.
- The CLI never caches the token, so it keeps working after a dashboard restart. A missing, unreadable or rejected token gives an error that says to start or restart `jidoka dashboard`; no error or response contains the token.
- The dashboard exits when it cannot write the token file or make its folder private. A second dashboard that fails to bind leaves the running one's file alone.
- An older CLI with a newer dashboard, or the reverse, fails with 401 on POST until the dashboard is restarted with the updated CLI.
- The CLI reads at most a few bytes and refuses a token file that is not a regular file, so a named pipe cannot block it ([PR #306](https://github.com/TDSchreur/jidoka/pull/306)). A path swapped to a pipe between the check and the open could still block; anyone who can swap that path can already tamper with the token file.
