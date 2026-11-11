# Runs and the dashboard

How the dashboard shows your runs, and the `jidoka run` commands that agents and you use to record them.

## The dashboard

Start the dashboard and keep it running in a separate terminal: `jidoka dashboard`. Open <http://127.0.0.1:4311> to follow your runs. The dashboard follows your system's light or dark setting; use the Theme control (System, Light, Dark) at the bottom of the page to override it. The choice is remembered in your browser. While you scroll, the andon strip (what needs you) and the active runs stay at the top of the page. When the dashboard server restarts, for example after an update, an open dashboard page reloads itself within a few seconds, once no dialog or menu is open, so it always shows the running version.

One dashboard serves every Project. Running `jidoka dashboard` while one is already running prints its URL (opening on the current Project) and exits, and warns when that dashboard is a different build; it never stops it. Use `jidoka dashboard --port <port>` for a separate dashboard on another port with its own `runs-<port>.json`; `jidoka run` reports to the default port only.

## Starting a run with its workspace

`jidoka run begin "<title>"` creates a run and its workspace in one command. The workspace is a git worktree on a new branch, made from the pull request base branch. It replaces `jidoka run start`, the `git worktree add` and the `jidoka run update` that records them; `jidoka run start` still works as before.

```text
jidoka run begin "<title>" [--agent <codex|claude|copilot>] [--mode <jidoka-full|jidoka-balanced|jidoka-fast>]
                  [--base <branch>] [--branch <name>] [--json]
jidoka run begin --run <run-id> [--base <branch>] [--branch <name>] [--mode ...] [--agent ...] [--json]
```

- **Base.** `--base` names the branch. Without it the command uses the branch `refs/remotes/origin/HEAD` points to. It fetches that branch from `origin` and creates the worktree from `origin/<base>`; the recorded base commit is that commit. It does not read the project's documentation, so an agent passes the project's documented base itself.
- **Names.** `--branch` is used as given. Without it the branch is a kebab-case name from the title (ASCII letters and digits, at most 40 characters), with `-2`, `-3` and so on added until neither a branch nor a folder uses the name. The worktree is `<repo-parent>/<repo-name>.worktrees/<branch with / replaced by ->`, beside the main checkout, wherever the command runs. An explicit branch or folder that exists stops the command, and the retry command it prints leaves `--branch` out, so the retry picks a free name and never takes over what exists.
- **Recorded.** Mode, worktree, branch and base commit, the repository (as `run start` records it) and the agent session, with the stage set to define and the activity "Workspace ready". It prints the run ID first, then worktree, branch, base and base commit; `--json` prints the same, plus `gitignore_added`, `gitignore_warning` and `session_warning`.
- **One session per run.** When the current agent session (`CLAUDE_CODE_SESSION_ID`) is already recorded on another run, of any status, the output carries a `Warning:` line naming that run's id and title and advising a fresh agent session or `/clear` for each run (`--json`: `session_warning`, null otherwise). A resumed run is not counted against itself, and a failure to list runs gives no warning.
- **Ignored folders.** After the run is recorded, the worktree's `.gitignore` gets `.jidoka/` and `.scratch/` when they are missing (the file is created if absent). It is left uncommitted: the output lists the added entries (`--json`: `gitignore_added`), and the agent commits `.gitignore` with its first commit. When the file cannot be updated (for example it is a symlink or unreadable), the output carries a warning (`gitignore_warning`) and the run continues.
- **Failures.** Once the run exists, a missing `origin`, an unknown base, a failed fetch, git that cannot write, or a name that is taken mark the run blocked with the reason, print the reason and the command to retry, and exit with status 1. Outside a repository, inside another `.git` folder (a submodule), or with a bad option, no run is created. Nothing is ever deleted, reset, checked out over or cleaned.
- **Retry.** `jidoka run begin --run <id>` resumes that run. The command to retry that a failure prints carries `--branch <name>` as soon as a name was chosen; a resume takes over an existing branch or worktree only when it is given that branch, and without one it picks a free name like a new run, so it never takes over another run's workspace. If nothing was made it does everything; a branch without a worktree gets its worktree; a clean worktree that was made but not recorded is recorded. Anything else (a folder that is not a worktree, a branch with commits of its own, a worktree with uncommitted changes) stops and says exactly what it found. A worktree that git lists but whose folder is gone is reported, and you run `git worktree prune` yourself. A run that is finished, or already has a workspace, is refused.
- **Dedicated worktrees.** When your agent environment hands you a fresh worktree, the Workspace procedure still uses `jidoka run start` and `jidoka run update` by hand; `begin` always makes its own worktree.

## Reporting runs manually

For manual tracking, `jidoka run start "Task title"` prints a run ID (add `--agent claude`, `--agent codex` or `--agent copilot` to say which agent the run is in; model routing needs it when the policy file covers several agents), and `jidoka run update <id> build running` updates it. Stages are `define`, `build`, `check`, and `finish`; statuses are `running`, `blocked`, and `done`.

### Run context

Besides its stage and status, a run records the context a new session needs to continue it. It is recorded only through `jidoka run update`, never by editing the storage file, and holds references and short summaries, not transcripts or secrets:

| Option | Records |
| --- | --- |
| `jidoka run start` | The canonical `repository` (the main checkout, the same from every linked worktree) next to the folder the run started in. |
| `--worktree <absolute-path>`, `--branch <name>`, `--base-commit <sha>` | Where the run's work lives and the commit it started from. |
| `--mode <jidoka-full\|jidoka-balanced\|jidoka-fast>` | The workflow mode. |
| `--spec <http(s)-url\|path>`, `--pr <http(s)-url>` | The spec (a URL or a local path) and the pull request. URLs with credentials are refused. |
| `--ticket "<reference> <title>"` (repeat), `--ticket-status <reference>=<pending\|in-progress\|done>` | The ordered ticket list, listed whole each time (statuses of kept tickets are preserved), and its progress. At most one ticket is in progress. |
| `--run-report <skill\|activity> "<text>"` (repeat) | Append a short report to the run, with the current stage and server time. |
| `--ticket-report "<reference>" <skill\|activity> "<text>"` (repeat) | Append a short skill or activity report to a listed ticket, with the current stage and server time. |
| `--summary <stage>=<text>` | A short summary per stage, kept after later stages begin. |
| `--blocking-reason <text>` | The status reason of a blocked run. |

Options left out keep their value, each has a `--clear-*` form (`--clear-tickets`, `--clear-summary <stage>`), and an inconsistent update is refused with the reason and changes nothing.

`jidoka run next <id>` prints what the run does now: the current stage, its instructions, the open gates, the allowed moves (next stage and Rework), the stored summaries and tickets, and the stage's routed model and effort (`--json` for the same as data). In Check it also lists the applicable and skipped checks.

`jidoka run gate <id> <gate> <pass|fail> [--evidence "<text>"]` records a result for the current stage and Rework. Record `verified` after full verification; the command captures the worktree's HEAD. For full mode, set the current ticket and record `tickets-verified` after each ticket. In Check, record `checks-recorded` after the applicable reviews and `no-actionable-findings` when their findings have been resolved. A later result for the same gate replaces the earlier result for advancement. Ticket completion and CI are checked automatically.

`jidoka run advance <id>` checks the current stage's gates and moves to the next stage, or marks Finish done. A failed gate leaves the run unchanged and gives the command to resolve it. Verification must match the current HEAD, and Finish requires an open pull request whose head matches the worktree and whose CI passed or has no checks. CI is read from GitHub or Azure DevOps. If no checks appear, the command waits through the 90-second discovery period before accepting that the project has no CI. During this transition period, `run update` can still change the stage directly.

Run storage now includes gate results. Existing stored runs are not migrated: finish or abandon in-flight runs before upgrading.

`jidoka run inspect <id>` prints a run's context (`--json` for the stored record). The dashboard shows a run's detail as a route card that follows the run along Define, Build, Check and Finish (the worktree and base commit stay in `jidoka run inspect`), shows ticket progress and pull request and spec links on each run's row, and counts the runs of every linked worktree under their repository. A local spec or ticket (a recorded reference that is not an http(s) link, for example from the local tracker) gets a View link, and a local ticket's title opens it, in the Reader: a dialog over the detail that shows the Markdown file rendered. Agents record such references as absolute paths so the dashboard can find them.

To continue a run in a new session, ask the agent to continue it by ID. The workflow skills then read the context and check the recorded repository, worktree, branch, base commit and Git state, report any difference, and never reset, clean or discard work to make them match.

`jidoka run check <run-id>` compares what the run records with the repository, so a new session can see at once whether it is safe to continue. It only reads: it never fetches, resets, cleans, checks out, deletes or commits. Git runs without optional locks and without lazy fetches in a partial clone, and with `core.fsmonitor` and `log.showSignature` turned off, so the fsmonitor hook and signature checks do not run. Clean and process filters that the repository sets can still run during `git status`, as they would for any `git status` there, so run the check only on repositories you trust.

```text
jidoka run check <run-id> [--json]
```

It runs five checks and reports each as `matches`, `differs` with what it found, or `unknown` when the run never recorded the field. A check that needs the worktree is `not-checked` when the worktree is missing or differs. A git command that fails during a check also counts as `differs`, with git's message.

| Check | Matches when |
| --- | --- |
| `repository` | The recorded repository exists, and `git rev-parse --git-common-dir` gives the same directory there as in the worktree. |
| `worktree` | The recorded worktree exists and `git worktree list --porcelain` of the recorded repository lists it. |
| `branch` | `git branch --show-current` in the worktree is the recorded branch. A detached HEAD differs. |
| `base-commit` | The recorded base commit is a commit id (4 to 64 hexadecimal digits), the repository has it, and `git merge-base --is-ancestor <base commit> HEAD` succeeds. |
| `working-tree` | `git status --porcelain` is empty. Otherwise the changed paths are listed. |

Commits of `HEAD` that no remote-tracking branch has (as of the last fetch) are listed as not pushed, which does not count as a difference. A repository without remote-tracking branches says the list is not known. The exit status is 0 when nothing differs, also when some fields are unknown, 2 when a check differs, and 1 for a problem such as bad arguments or an unreachable dashboard. `--json` prints the same data (`run`, `consistent`, `exit_code`, `checks` with `check`, `state`, `recorded` and `found`, `changes`, `unpushed` and, when the list is not known, `unpushed_unknown`).

`jidoka run sync <run-id>` reads the state of the run's pull request from its host (`gh` for GitHub, `az` for Azure Repos) and brings the run in line with it: a merged pull request clears the activity (such as "Waiting for CI") and marks Finish done, a closed one clears the activity and blocks the run with "PR closed without merge", and an open one changes nothing. A host that cannot be reached is reported and leaves the run as it is. The dashboard does the same on its own for every run with a pull request, and the run sheet has a "Check PR" button to do it at once. `jidoka pr wait` does it when it reports that the pull request is no longer open.

```text
jidoka run sync <run-id> [--json]
```

`--json` prints `run`, `pull_request`, `state` and `run_updated`.

`jidoka run cleanup <run-id>` removes the worktree and local branch of a run after its pull request merged, so you do not clean up by hand. Run it from the main checkout, not from inside the worktree. A directory at the recorded worktree path that git does not list as a worktree is left alone: the command says nothing was removed and names it, and the run's record is still cleared.

```text
jidoka run cleanup <run-id>
```

It reads the pull request state fresh and refuses, with exit status 1 and nothing removed, unless the pull request is merged. It also refuses when the run records no pull request, when the pull request comes from another branch than the run's recorded branch, when the run records a worktree but no branch (nothing ties that worktree to the pull request), when the worktree is detached or on another branch, when the worktree has uncommitted changes, when the local branch is not at the head commit of the merged pull request (it may hold commits that were never pushed), when your current directory is inside the worktree, when the worktree is locked, when the branch is checked out in any other worktree (the main checkout included; checked before anything is removed), and when the recorded repository or worktree path is relative. Otherwise it runs `git worktree remove` (without `--force`; for a worktree directory you deleted by hand it drops only that registration, and other stale worktrees are left alone) and checks that git no longer lists it (a recorded worktree directory that exists but is not registered with git is reported as left alone, not removed). The worktree is deleted with everything in it, gitignored files such as `.env` or build output included, so copy out what you need first. Then it deletes the branch with `git update-ref -d`, pinned to the commit it checked (a squash merge needs a forced delete, and a branch that moved since the check is kept), in the run's repository. The remote branch is left alone. The run stays on the dashboard with its worktree and branch cleared and the activity "Cleaned up worktree and branch"; its stage and status are kept. Running it again finishes what is left and reports what was already gone.

Run updates also accept optional reporting fields:

```sh
jidoka run update <id> build running --activity "Implementing ticket #42" --current-ticket "https://github.com/owner/repo/issues/42"
jidoka run update <id> build running --ticket-report "https://github.com/owner/repo/issues/42" skill "jidoka-tdd" --ticket-report "https://github.com/owner/repo/issues/42" activity "Tests passed"
jidoka run update <id> build blocked --blocking-reason "Waiting for credentials"
jidoka run update <id> build running --clear-blocking-reason
jidoka run update <id> build running --rework 1 --activity "Fixing Check findings"
jidoka run update <id> finish done --summary finish="<what shipped>" --clear-activity --clear-current-ticket
```

Omitted fields keep their stored values, including when changing stage or status. Use `--clear-activity`, `--clear-current-ticket`, `--clear-blocking-reason`, and `--clear-rework` to remove them explicitly. Setting and clearing the same field in one command, or repeating a field option, is an error. Activity and blocking reason must be nonblank and at most 2,000 bytes; a current-ticket reference must be an HTTP(S) URL, local path or identifier of at most 4,096 bytes without spaces. Empty strings are rejected; use a clear flag instead. `--rework` records which [Rework](../GLOSSARY.md) the run is on, as a whole number from 1 to 99; the dashboard shows it as "Rework 1/3", or "Rework 4 (past the cap of 3)" when the user granted an extra Rework.

The API uses `activity`, `current_ticket`, `blocking_reason`, and `rework` on `POST /api/runs/<id>` alongside the required `stage` and `status`. Omit a metadata field to preserve it, send a value (a string, or a number for `rework`) to replace it, or send JSON `null` to clear it. Unknown update fields are rejected. For example:

```json
{"stage":"build","status":"running","activity":"Implementing ticket #42","blocking_reason":null}
```

Run details list every pass through a stage (each Build and Check, including Reworks) with its start time, duration and the activity it started with. Run reports are displayed in the last pass of the stage they were filed under that had started by then (the nearest pass of that stage otherwise; by time for older runs that recorded no stage), including when no tickets exist. Ticket reports are displayed under their ticket in run details when that pass lists the ticket, and in the pass otherwise. When a run is blocked or resumed, run details show a "Blocked: <reason>" or "Resumed" line in the pass that holds it; `jidoka run inspect` lists them under "Events" (`events` in `--json`); events that match no pass show under "Other reports". Pauses the agent's log shows (see [Pauses](usage-tracking.md#pauses)) and idle gaps appear the same way. They are agent-reported and may be incomplete; no reports does not mean no work occurred. Each report is limited to 500 bytes; run and ticket reports are each limited to 100 per update and 1,000 per run. Do not report secrets or transcripts.

Run responses and `runs.json` include these optional fields and `updated_at`, a server-owned Unix timestamp in milliseconds set on creation and each successful update. Callers cannot set `updated_at` through an update. Existing records load with `null` for missing metadata and timestamps; they receive a timestamp on their next update. The timestamp records a run update, not an agent heartbeat. Restart the dashboard with the updated CLI before using the new options. The dashboard shows these fields on each run's row on the line. Finished runs (status `done`) are listed under Shipped, grouped by day. Next to each pull request link the dashboard shows its state (Open, Draft, Merged or Closed). The dashboard server reads it in the background, with `gh` for GitHub and with `az` and its `azure-devops` extension for Azure Repos, stores it on the run as `pr_status`, and reads it again about once a minute until the pull request is merged. Without the tool, or when it is not signed in, the state is unknown and the run's detail says why. Callers cannot set `pr_status`. A run is deleted from its detail, and Clear shipped removes all shipped runs, each after a confirmation; deleting removes only the run record, not its worktree or pull request. The record is removed by `DELETE /api/runs/<id>`, which needs the session token from the dashboard page (sent as `X-Session-Token`), so it works from the dashboard only.

`POST /api/runs` and `POST /api/runs/<id>` need the CLI token, sent as `X-Jidoka-Token`. The dashboard page's session token is refused there, so reading the page grants no write access. A missing or wrong CLI token gets 401; a refused Host, Origin or content type gets 403. The dashboard writes a new CLI token on each start to `session-token` (`session-token-<port>` for another port) next to `runs.json` in the data folder above. On Linux and macOS the file is mode 0600, and the folder is made 0700 when the dashboard creates or owns it (a symlinked folder is left as it is, with a warning on stderr when its target is accessible to group or others; run `chmod 700` on it; a dangling symlink stops the dashboard from starting); if it cannot be made private the dashboard fails to start; on Windows the file relies on the per-user access rules of `%LOCALAPPDATA%`. The `jidoka` CLI reads the file on each request, so it keeps working across dashboard restarts. If the file is missing or the server answers 401, the CLI says to start or restart `jidoka dashboard`. If the dashboard cannot write the file, it prints the path and the error and exits with a failure. The token never appears in an error or a response. After upgrading, restart the dashboard with the updated CLI: an older dashboard or CLI on either side makes POSTs fail.

## Security

The server listens only on localhost but has no user authentication. Other users on the same machine can read and change run titles and project paths; use it only on a trusted machine.
