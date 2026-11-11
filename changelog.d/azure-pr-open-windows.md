## Fixed

- `jidoka pr open` on Azure Repos no longer fails with "Could not start Azure CLI" on Windows for a multi-line body: the body is passed to `az` as `@<file>`. The error now names the program and the cause, and an over-long body reports its length and how much to cut.
