---
status: accepted
---

# One remote parser per host accepts https, http and ssh and ignores the port

Jidoka had two ways to tell whether a git remote is a github.com repository, and they disagreed. `pr_host::is_github_remote` (used by `jidoka pr open` and `jidoka doctor` to pick the pull request host) took https, http, ssh and scp-style remotes, but only with no port or port 22 or 443. The knowledge repo's own `github_repo` parser (used to build web links to change specs) took any scheme, including `git://`, and any port. We decided that each host has one remote parser in the pull request host module, `GitHubRepo::from_remote` next to `AzureRepo::from_remote` (now in `src/pr/host/`), and that every caller uses it. The GitHub parser accepts `https://`, `http://`, `ssh://` and scp-style `git@github.com:` remotes and ignores the user and port. This ADR records [PR #613](https://github.com/TDSchreur/jidoka/pull/613) after the fact: that pull request made the decision without one.

## Considered options

- **Keep two parsers.** Rejected: the same remote could be GitHub for one command and not for another, and every fix had to be made twice.
- **One parser that also accepts `git://` and other schemes.** Rejected: GitHub stopped serving the unauthenticated `git://` protocol in 2022, and `jidoka pr open` cannot work with a scheme like `file://`. Accepting them would make host detection claim remotes it cannot use.
- **One parser that rejects unusual ports, as `is_github_remote` did.** Rejected: the port says how git connects, not which repository it is. Owner and repository are the same, so a link or a `gh` call works either way, and rejecting the remote only produced an error.

## Consequences

Because the two old checks could not both be kept, two edge cases changed:

- `jidoka pr open` and `jidoka doctor` now recognize a github.com remote with an unusual port, such as `ssh://git@github.com:2222/acme/app.git`. Before, this failed with "Cannot identify the repository provider".
- A knowledge repo whose origin is a `git://github.com/...` remote no longer gets web links to its change specs. Its origin needs an https or ssh URL.

GitHub Enterprise remotes are still not recognized by host name; they need `git config jidoka.prProvider github`, as before. A new host gets its own `from_remote` in the same module, used by every caller.
