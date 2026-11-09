## Changed

- The dashboard browser tests rerun once, with the reason logged on stderr, when the browser crashed, exited or gave no result in time; a failed check still fails at once. Each attempt has its own server and result queue, so a late result from a timed-out first browser is never taken as the retry's result.
