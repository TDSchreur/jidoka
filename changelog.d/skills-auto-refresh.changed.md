## Changed

- Jidoka refreshes the installed skills automatically on the first command after it is upgraded, without prompting.
- An unreadable install record now prints a warning naming the file and the fix (`jidoka install`). The record is written atomically, and a stale refresh lock is taken over more safely.
