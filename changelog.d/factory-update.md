## Added

- `factory update` installs the latest release over the running `factory`. It downloads the release with `curl`, checks the SHA-256 checksum before replacing anything, and then reinstalls the skills for the agents that already have them. `--agent` picks the agents instead.
