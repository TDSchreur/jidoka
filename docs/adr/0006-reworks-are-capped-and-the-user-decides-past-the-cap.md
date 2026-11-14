---
status: accepted
---

# Check sends findings back to Build as a capped Rework

Check used to fix findings in place and, when one could not be resolved, mark the run blocked and stop before Finish. There was no explicit return to Build, no limit on fix rounds, and the dashboard could not show that a run was reworking ([issue #35](https://github.com/TDSchreur/jidoka/issues/35)). We decided that actionable findings send the run back to Build for a **Rework** using the mode's build approach, followed by another Check. The cap started at two Reworks ([PR #38](https://github.com/TDSchreur/jidoka/pull/38)) and is now three ([PR #258](https://github.com/TDSchreur/jidoka/pull/258)); the PRs do not record why it was raised. Past the cap the agent asks the user to choose: one more Rework, finish with the findings listed in the pull request, or stop. The Fast mode has no Check and no Reworks.

After the second Rework, findings that change only wording or documentation are fixed in Rework 3 without asking, and findings that change behaviour need the user's decision ([PR #259](https://github.com/TDSchreur/jidoka/pull/259)). Wording means comments, docs, docstrings, log messages and typos in internal names. Behaviour means user-facing messages, public names and interfaces, outputs and logic. Without the definitions, agents fixed a wrong error message and a misspelled public function name without asking in both of two trial runs each; with them they asked.

## Considered Options

- **Block the run on an unresolved finding (the previous behaviour).** Rejected: work stops without the user having a say.
- **Fix in place inside Check, without a return to Build.** Rejected: fixes were not built with the mode's test approach, and the dashboard could not show a fix round.
- **No cap.** Rejected: a run could loop indefinitely.

## Consequences

- Findings alone never block a run; it is `blocked` only while it waits for the user's decision past the cap.
- The cap exists in two places that must be kept in step: the Rework rules in the procedures skill and `REWORK_CAP` in `dashboard/app.js`, which shows "Rework 1/3" and "past the cap of 3" from the fourth.
- The wording-versus-behaviour split is judged by the agent from the definitions above, so a borderline finding can go either way.
