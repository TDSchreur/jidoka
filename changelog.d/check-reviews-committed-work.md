## Fixed

- `/factory` and `/factory-balanced` now commit the finished implementation, and each Rework, before Check, so `code-review` always reviews the complete change, including new files. The first review covers everything since the run's base commit; follow-up reviews cover the commits since the previously reviewed one. Finish no longer relies on a late first commit, and CI fixes still use new commits, never amend or force-push.
