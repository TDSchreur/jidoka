## Changed

- Build requires listed tickets or an explicit `jidoka run update <id> --unticketed` declaration. Use `--ticket-status <reference>=in-progress` to activate a listed ticket and `done` or `pending` to leave it. The `--current-ticket` and `--clear-current-ticket` flags and the `current_ticket` reporting field are removed.
- Full Build records the verified commit and work attempt. Reopened or re-listed tickets need a new pass, while unaffected tickets can keep their evidence across Rework. Full unticketed Build verifies its current commit. Older ticket passes without this evidence must be recorded again if the run returns to Build.
