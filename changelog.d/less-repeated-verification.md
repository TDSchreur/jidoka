## Changed

- `/factory-balanced` and `/factory-fast` now run the project's relevant checks while implementing and its full verification once on the finished candidate, instead of after every ticket. `/factory` still runs it after each ticket. Any workflow reruns it when code changed since the last passing run (after a Rework or a CI fix), and Finish reuses the passing result for an unchanged commit instead of repeating it. Stricter project-required checks always apply, and a missing tool or failing check still blocks the run.
