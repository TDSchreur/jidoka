## Added

- Add change groups: `jidoka group create|list|inspect|add|remove|delete` link ordinary runs, in any repository, to one shared spec and show their recorded pull request states. `jidoka run inspect` shows a run's group, and the dashboard refuses to delete a run that is in a group until you detach it with `jidoka group remove`. Groups live in a new `groups.json` in the data folder; restart the dashboard after upgrading.
