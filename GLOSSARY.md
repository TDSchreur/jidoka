# Glossary

**Run**: One piece of work carried through the factory flow.

**Stage**: The current part of a run: Define, Build, Check, or Finish.

**Needs you**: A run that is blocked and waiting for the user. The dashboard marks its card, sorts it first and counts it in the page title.

**Rework**: A return from Check to Build to fix actionable findings, followed by another Check. `factory` and `factory-balanced` allow at most two per run; after that, the user decides whether to do another Rework, finish with the findings listed in the pull request, or stop. Check findings never block a run.

**Workflow mode**: The workflow skill that carries a run: `factory` (quality), `factory-balanced`, or `factory-fast` (speed). Modes differ in how much Define, Build, and Check do.
