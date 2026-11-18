---
status: accepted
---

# The knowledge repo is a per-user setting and is not worked through runs

The knowledge repo (platform knowledge and cross-repo change specs) is found through `knowledge.json` in Jidoka's per-user config folder (`$XDG_CONFIG_HOME/jidoka/` or `~/.config/jidoka/` on Linux, `%APPDATA%\jidoka\` on Windows), next to the model-routing policy. It holds the knowledge repo checkout path and the machine-local checkout path of each repository. Agents commit and push to the knowledge repo's default branch directly; it is never a Jidoka run, and `jidoka run next` only tells agents to read it in Define and record learnings in Finish.

## Considered options

- **A per-project setting** (in the repository or `git config`): a platform has one knowledge repo that every repository's runs should use, so it would be repeated and drift. Rejected.
- **Committing the local checkout paths in the knowledge repo**: paths differ per person and machine, so committing them breaks teammates. They stay in the per-user file; the repo lists only name, host, remote, role, owner and default branch.
- **Working the knowledge repo through runs** (worktree, review, pull request per note): updates are small and frequent, so the cost would stop agents from recording anything. Direct pushes with pull-rebase-retry and one note per commit are enough for small, rarely conflicting notes. The price is that nobody reviews a note before it lands; a later review curates `learnings/`.
- **Storing the setting in the dashboard's data folder** (`runs.json` area): it needs the dashboard to read it, but `jidoka run next` and `jidoka group create` also run with the dashboard's address only for run data. A plain file read by the CLI keeps `jidoka knowledge` working without a dashboard.
- **Keeping the interim `knowledge.md` the stage A skills wrote**: free-form Markdown that the CLI would have to parse, and agents edited by hand. Replaced by JSON managed through `jidoka knowledge`; no migration on 0.x.

## Consequences

`jidoka group create --spec <file in the knowledge repo>` builds the web link from the repo's `origin` and default branch and refuses a file that is not committed and on `origin/<branch>`. Only GitHub and Azure DevOps remotes have links. Without the setting, `jidoka run next` and `group create` behave as before. Anyone who used the interim `knowledge.md` runs `jidoka knowledge set` and `jidoka knowledge checkout` once.
