## Added

- Add the `jidoka-pr-watch` skill. It watches your own open, non-draft pull requests in the current repository on an interval (10 minutes by default) on GitHub and Azure DevOps. Each pass rebases a branch with merge conflicts onto its base and force-pushes it with a lease, fixes review comments that make sense (verified, pushed, replied to and resolved), and answers the ones it declines with a short, dry reply that gives the reason. Questions that need your decision are reported to you instead. It never merges a pull request.
