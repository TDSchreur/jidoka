## Changed

`jidoka pr wait` asks the host again when a call fails, up to 3 more times, 10 seconds apart and within `--timeout`, instead of ending with exit status 1 at the first failure.
