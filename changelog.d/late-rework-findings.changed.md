## Changed

- `jidoka run update --finding` also records Findings in Build during a Rework, so a check result that arrives after the Rework started joins the current Rework instead of sending the run back to Check. The Check procedure now waits for every started check before starting a Rework.
