## Changed

- `scripts/verify.sh` skips the dashboard browser tests on WSL2 and when `JIDOKA_SKIP_BROWSER_TESTS=1`, and says so. On other systems a missing browser is still a blocker.
- The git tests no longer fail when the machine's git sets `safe.bareRepository=explicit`.
