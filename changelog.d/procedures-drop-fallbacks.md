## Changed

- The workflows no longer redo `factory run begin`, `run check`, `pr open`, `pr wait` or `pr sync` by hand when the installed `factory` lacks one of them. The skills and the binary are installed together, so a missing command means they are out of step: the run reports that the factory needs upgrading with `factory install`, is marked blocked, and stops.
- The Pull request and CI procedure is split into short sections (opening the pull request, watching CI, keeping the branch current, when CI fails, reporting the result), and three of its rules now say why: no amending or force-pushing, no changes in the main checkout, and no merging.
