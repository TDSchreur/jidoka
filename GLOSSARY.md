# Glossary

**Run**: One piece of work carried through the factory flow.

**Stage**: The current part of a run: Define, Build, Check, or Finish.

**Needs you**: A run that is blocked and waiting for the user. The dashboard marks its card, sorts it first and counts it in the page title.

**Rework**: A return from Check to Build to fix actionable findings, followed by another Check. `factory` and `factory-balanced` allow at most two per run; after that, the user decides whether to do another Rework, finish with the findings listed in the pull request, or stop. Check findings never block a run.

**Workflow mode**: The workflow skill that carries a run: `factory` (quality), `factory-balanced`, or `factory-fast` (speed). Modes differ in how much Define, Build, and Check do.

**Run context**: What a new agent session needs to continue a run: the repository, worktree, branch and base commit it started from, the workflow mode, a reference to the spec, the ordered tickets with their progress, the pull request, a short summary per Stage and the status reason. It holds references and summaries only, never transcripts or secrets, and is changed only through `factory run update`. Anything a run never recorded is shown as unknown, not guessed.

**Repository**: The canonical main checkout of the project a run belongs to. Every linked worktree of a checkout has the same repository, so the dashboard groups their runs in one row.

**Status reason**: Why a run is blocked, in the run's own words. It is the blocking reason a run is blocked with.

**Continuing a run**: Starting a new session on an existing run. The session reads `factory run inspect <id>`, checks the recorded repository, worktree, branch, base commit and Git state, reports any difference to the user and never resets, cleans or discards work to make them match.
