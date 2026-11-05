## Changed

- `/factory`, `/factory-balanced` and `/factory-fast` now read their shared procedures (run reporting, continuing a run, workspace, tracker setup, and pull request and CI) from one installed `factory-procedures` skill instead of repeating them in each skill. Each workflow keeps its own planning, testing, review and verification rules, and behaves as before. `factory install` installs the new skill for every supported agent.
