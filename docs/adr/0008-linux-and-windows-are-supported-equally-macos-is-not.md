---
status: accepted
---

# Linux and Windows are supported equally; macOS is not

Jidoka has users on Linux and on Windows, and none on macOS. We decided that Linux and Windows get the same level of support, and that macOS is not a supported platform. This follows the rule in [ADR-0007](0007-gitlab-is-not-a-supported-issue-tracker.md): we support what the maintainer and the users use, and no more, because we cannot support everything.

In practice, the CI checks and the release build run on Linux and Windows only, and code that starts `az`, `gh`, `git` or other programs has to work on both ([external tools guide](../../.github/instructions/external-tools.instructions.md)).

## Considered Options

- **Linux first, Windows best effort.** Rejected: Windows users are real users, so a change that works only on Linux is not finished.
- **Also support macOS.** Rejected: nobody uses it, so there is no one to test with or fix for, and it would add a third platform to keep at parity.

## Consequences

- A change that cannot work on one of Linux and Windows needs a discussion with the user and its own ADR, like the other parity rules.
- Some code and docs still mention macOS, for example the Unix file modes in the data folder and the dashboard browser tests. That is shared Unix behaviour, not a promise: nothing tests macOS and a macOS-only bug is not a bug we fix. Add a macOS user, a macOS CI job and a new ADR before relying on it.
