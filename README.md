# TDS Factory

An early Rust foundation for a small software factory, inspired by the original [TDS Factory](https://github.com/TDSchreur/tds-factory). The goal is to take a piece of work from idea to delivery with a simple flow and low time and token costs.

The core flow is **define → build → check → finish**, implemented as an agent skill. Start small and add steps only when they are useful.

## Install

Prerequisites: Linux x86-64 or Windows x86-64, `git`, and a supported coding agent (Codex, Claude Code, or GitHub Copilot). To open pull requests and watch CI, the workflows need the `gh` CLI on GitHub, or the Azure CLI (`az`) with its `azure-devops` extension on Azure Repos. Building from source also needs a Rust toolchain with Cargo.

Install the latest release with one command:

Linux x86-64:

```sh
curl -fsSL https://github.com/TDSchreur/tds-factory2/releases/latest/download/install.sh | sh
```

Windows x86-64 (PowerShell):

```powershell
irm https://github.com/TDSchreur/tds-factory2/releases/latest/download/install.ps1 | iex
```

Both installers download the prebuilt CLI, verify its release checksum, ask which agents to install the TDS Factory skills for (Codex, Claude Code, GitHub Copilot, one or more), and install them. Answer with numbers or names, such as `1,3` or `claude copilot`, or `all`. To skip the prompt, set `FACTORY_AGENT` to a comma-separated list before running the installer, for example `FACTORY_AGENT=codex,claude` (`$env:FACTORY_AGENT = 'codex,claude'` in PowerShell). Without `FACTORY_AGENT` and without a terminal to ask on, the installer stops before installing anything. Each install replaces the bundled skill directories with the versions shipped in the CLI, so local edits to those skills are lost; other skills in the same folder are left alone. To install skills for other agents later, run `factory install --agent claude` (or `codex` / `copilot`, or a comma-separated list such as `codex,claude`). Re-running the one-line installer updates both the CLI and its skills.

To see whether a newer release exists, run `factory check`. It looks up the latest release with `curl`, says whether you are up to date, and exits 0 either way (1 on an error, such as no network). A development build cannot be compared, so it only names the latest release.

To update, run `factory update`. It downloads the latest release for your platform with `curl`, verifies its checksum, replaces the installed `factory`, and reinstalls the skills for the agents that already have them. Use `factory update --agent codex,claude` to choose the agents yourself. If you run a development build, it updates to the latest release. It supports Linux and Windows on x86-64 only. Restart a running dashboard afterwards. Re-running the one-line installer does the same.

Claude Code skills go to `$CLAUDE_CONFIG_DIR/skills` when `CLAUDE_CONFIG_DIR` is set (for example `~/.claude-work`), otherwise `~/.claude/skills`. Codex skills likewise honor `CODEX_HOME`, and Copilot skills honor `COPILOT_HOME` (otherwise `~/.copilot/skills`).

Stop the dashboard before upgrading when you can; `factory update` renames a running Windows executable aside, but the dashboard keeps running the old version until you restart it. The Windows CLI cross-compiles, but the installer has not yet been run on Windows; the release workflow tests the build on a Windows runner.

To build from source instead, run:

```sh
cargo run -- --help
cargo run -- --version
cargo run -- dashboard
```

The Rocket server binds to localhost at <http://127.0.0.1:4311>; press Ctrl+C to stop it. It stores run progress under `$XDG_DATA_HOME/tds-factory/` (or `~/.local/share/tds-factory/` when unset) on Linux, and `%LOCALAPPDATA%\tds-factory\` on Windows. Keep it running while using TDS Factory. After a merge, `scripts/update-local.sh` updates the checkout and prompts for one or more agents; set `FACTORY_AGENT=codex,claude` to select them without a prompt. To install from a checkout, use `cargo install --path . --locked`, ensure Cargo's bin directory is on PATH, and run `factory install --agent <codex|claude|copilot>`. Restart a previously running dashboard after upgrading.

One dashboard serves every Project. Running `factory dashboard` while one is already running prints its URL (opening on the current Project) and exits, and warns when that dashboard is a different build; it never stops it. Use `factory dashboard --port <port>` for a separate dashboard on another port with its own `runs-<port>.json`; `factory run` reports to the default port only.

To switch a Linux x86-64 development machine between the latest release and the current `origin/develop`, run `scripts/switch-factory.sh release` or `scripts/switch-factory.sh develop` from this checkout. The script fetches and builds `develop` in a temporary checkout without changing your working tree, stops the dashboard, removes the existing Cargo and `~/.local/bin` factory executables, and installs the selected version in `~/.local/bin`. It prompts for a comma-separated list of agents, or uses `FACTORY_AGENT` when set. Selected agents receive the bundled skills for that version. Restart the dashboard afterwards. Ensure `~/.local/bin` is on your PATH.

The dashboard page is plain files in `dashboard/` (`index.html`, `app.js`, `help.js`, `style.css`, `favicon.svg`), built with Preact and htm, committed in `dashboard/vendor/` and loaded through an import map, so there is no build step and no `npm install`. The server embeds the files in the binary and serves only the ones it lists under `/assets/`. When you add a file the page loads, add it to the list in `src/server.rs`. See [`dashboard/vendor/README.md`](dashboard/vendor/README.md) for the library versions; their licenses are in `third-party/`.

The dashboard footer and `factory --version` show which build is running: the version number, such as `0.3.1`, for an official release, and `dev <commit>`, such as `dev 1a2b3c4`, for any other build, including builds from a checkout. A build without git information, such as from a source archive, shows `dev (unknown commit)`. Official release builds set `FACTORY_RELEASE=1`.

## Use

1. Start the dashboard and keep it running in a separate terminal: `factory dashboard`. Open <http://127.0.0.1:4311> to follow your runs. The dashboard follows your system's light or dark setting; use the Theme control (System, Light, Dark) to override it. The choice is remembered in your browser.
2. In your project, ask your coding agent to run `/factory` to select a workflow, or choose `/factory-full`, `/factory-balanced`, or `/factory-fast` directly; see [Choosing a mode](#choosing-a-mode). The agent reports progress to the dashboard with `factory run start` and `factory run update`, and ends by opening a pull request and watching its CI.
3. Review the pull request. See [Skills](#skills) for what each workflow does.

### Reporting runs manually

For manual tracking, `factory run start "Task title"` prints a run ID (add `--agent claude`, `--agent codex` or `--agent copilot` to say which agent the run is in; model routing needs it when the policy file covers several agents), and `factory run update <id> build running` updates it. Stages are `define`, `build`, `check`, and `finish`; statuses are `running`, `blocked`, and `done`.

#### Run context

Besides its stage and status, a run records the context a new session needs to continue it. It is recorded only through `factory run update`, never by editing the storage file, and holds references and short summaries, not transcripts or secrets:

| Option | Records |
| --- | --- |
| `factory run start` | The canonical `repository` (the main checkout, the same from every linked worktree) next to the folder the run started in. |
| `--worktree <absolute-path>`, `--branch <name>`, `--base-commit <sha>` | Where the run's work lives and the commit it started from. |
| `--mode <factory-full\|factory-balanced\|factory-fast>` | The workflow mode. |
| `--spec <http(s)-url\|path>`, `--pr <http(s)-url>` | The spec (a URL or a local path) and the pull request. URLs with credentials are refused. |
| `--ticket "<reference> <title>"` (repeat), `--ticket-status <reference>=<pending\|in-progress\|done>` | The ordered ticket list, listed whole each time (statuses of kept tickets are preserved), and its progress. At most one ticket is in progress. |
| `--summary <stage>=<text>` | A short summary per stage, kept after later stages begin. |
| `--blocking-reason <text>` | The status reason of a blocked run. |

Options left out keep their value, each has a `--clear-*` form (`--clear-tickets`, `--clear-summary <stage>`), and an inconsistent update is refused with the reason and changes nothing. A run recorded before this existed shows its missing context as unknown.

`factory run inspect <id>` prints a run's context (`--json` for the stored record). The dashboard shows it in the run's detail, shows ticket progress and pull request and spec links on cards, and groups the runs of every linked worktree under their repository.

To continue a run in a new session, ask the agent to continue it by ID. The workflow skills then read the context and check the recorded repository, worktree, branch, base commit and Git state, report any difference, and never reset, clean or discard work to make them match.

`factory run check <run-id>` compares what the run records with the repository, so a new session can see at once whether it is safe to continue. It only reads: it never fetches, resets, cleans, checks out, deletes or commits. Git runs without optional locks and without lazy fetches in a partial clone, and with `core.fsmonitor` and `log.showSignature` turned off, so the fsmonitor hook and signature checks do not run. Clean and process filters that the repository sets can still run during `git status`, as they would for any `git status` there, so run the check only on repositories you trust.

```text
factory run check <run-id> [--json]
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

Run updates also accept optional reporting fields:

```sh
factory run update <id> build running --activity "Implementing ticket #42" --current-ticket "https://github.com/owner/repo/issues/42"
factory run update <id> build blocked --blocking-reason "Waiting for credentials"
factory run update <id> build running --clear-blocking-reason
factory run update <id> build running --rework 1 --activity "Fixing Check findings"
factory run update <id> finish done --clear-activity --clear-current-ticket
```

Omitted fields keep their stored values, including when changing stage or status. Use `--clear-activity`, `--clear-current-ticket`, `--clear-blocking-reason`, and `--clear-rework` to remove them explicitly. Setting and clearing the same field in one command, or repeating a field option, is an error. Activity and blocking reason must be nonblank and at most 2,000 bytes; a current-ticket link must be an absolute HTTP(S) URL of at most 4,096 bytes. Empty strings are rejected; use a clear flag instead. `--rework` records which [Rework](GLOSSARY.md) the run is on, as a whole number from 1 to 99; the dashboard shows it as "Rework 1/2", or "Rework 3 (past the cap of 2)" when the user granted an extra Rework.

The API uses `activity`, `current_ticket`, `blocking_reason`, and `rework` on `POST /api/runs/<id>` alongside the required `stage` and `status`. Omit a metadata field to preserve it, send a value (a string, or a number for `rework`) to replace it, or send JSON `null` to clear it. Unknown update fields are rejected. For example:

```json
{"stage":"build","status":"running","activity":"Implementing ticket #42","blocking_reason":null}
```

Run responses and `runs.json` include these optional fields and `updated_at`, a server-owned Unix timestamp in milliseconds set on creation and each successful update. Callers cannot set `updated_at` through an update. Existing records load with `null` for missing metadata and timestamps; they receive a timestamp on their next update. The timestamp records a run update, not an agent heartbeat. Restart the dashboard with the updated CLI before using the new options. The dashboard shows these fields on each run card. Done runs show on one line with a Show details toggle, and can be deleted one by one or all at once after a confirmation; deleting removes only the run record, not its worktree or pull request. The record is removed by `DELETE /api/runs/<id>`, which needs the session token from the dashboard page (sent as `X-Session-Token`), so it works from the dashboard only.

The server listens only on localhost but has no user authentication. Other users on the same machine can read and change run titles and project paths; use it only on a trusted machine.

## Skills

The workflow skills [`factory-full`](.agents/skills/factory-full/SKILL.md), [`factory-balanced`](.agents/skills/factory-balanced/SKILL.md), and [`factory-fast`](.agents/skills/factory-fast/SKILL.md) coordinate the flow using skills from [mattpocock/skills](https://github.com/mattpocock/skills) and [github/awesome-copilot](https://github.com/github/awesome-copilot), plus [blader/humanizer](https://github.com/blader/humanizer), which edits the prose they write. After installing the skills, ask your coding agent to run one of them on your task. See [skills/README.md](skills/README.md) for the upstream origins.

### Workflow modes

TDS Factory has three workflow modes that trade speed against rigor: `/factory-full` favors quality, `/factory-fast` favors speed, and `/factory-balanced` sits between them. All three report progress to the dashboard and end with the same Finish stage; `/factory-fast` skips Check. Each run uses its own fresh git worktree based on `origin/<base branch>`, so several runs can proceed at once. Start the run in a fresh, dedicated worktree if your agent environment provides one, or the workflow creates one next to the repository. The original checkout is not modified, and the worktree is kept after the pull request opens until you remove it with `git worktree remove`.

| Stage | `/factory-full` (quality) | `/factory-balanced` | `/factory-fast` (speed) |
| --- | --- | --- | --- |
| Define | Always grills with [`grill-with-docs`](skills/grill-with-docs/SKILL.md) (`grilling`, `domain-modeling`), writes a spec with [`to-spec`](skills/to-spec/SKILL.md), and breaks it into tickets with [`to-tickets`](skills/to-tickets/SKILL.md) | Uses a clear request as the spec, otherwise one `to-spec` pass without grilling (tracker setup only when it reads or publishes tracker specs or tickets); always writes explicit acceptance criteria; `to-tickets` only for multi-piece work; `grill-with-docs` only when ambiguity blocks implementation | Uses a clear request as the spec with a short plan and acceptance criteria, no tracker setup for a clear request; `grill-with-docs`, `to-spec`, and `to-tickets` only when needed |
| Build | [`implement-spec`](skills/implement-spec/SKILL.md), test-first with `tdd` red-green cycles | [`implement-spec-balanced`](skills/implement-spec-balanced/SKILL.md): implements each ticket, then adds behavior tests through public interfaces using `tdd`'s test-quality rules, without red-green cycles or seam approval | [`implement-spec-fast`](skills/implement-spec-fast/SKILL.md): implements directly; tests only where useful |
| Check | [`code-review`](skills/code-review/SKILL.md), then [`security-review`](skills/security-review/SKILL.md); findings go back to Build for a Rework, then both reviews run again over the fixes | One initial `code-review`, with follow-up reviews of the Rework fixes; `security-review` only when the diff touches input parsing, auth or secrets, network, file or process handling, or dependencies; findings go back to Build for a Rework | Skipped |
| Finish | [`pr`](skills/pr/SKILL.md) writes the description and opens a pull request against the selected base branch, then the agent watches its CI, fixes failures with new commits and merges the base branch in when the pull request falls behind, until it is up to date and green | Same | Same |

`/factory-fast` saves time through lighter planning, selective test writing and no Check stage, but it still runs the project's verification and waits for green CI before it reports the pull request ready. All modes build sequentially in one worktree without subagents and run the project's relevant checks as they go. Full verification, meaning every command the project requires before merging, runs after each ticket in `/factory-full`, but once on the finished candidate in `/factory-balanced` and `/factory-fast`; `/factory-full` and `/factory-balanced` also rerun it after a Rework. Finish verifies the final commit, reusing the latest passing full verification when nothing has changed since it ran. Stricter project-required checks always apply. The three workflows share their run reporting, continuation, workspace, tracker setup, writing and pull request procedures in the installed `factory-procedures` skill, which they read before starting; each workflow keeps its own planning, testing, review and verification rules. Prose for people, such as specs, tickets, summaries and pull request descriptions, goes through the bundled `humanizer` skill, which leaves facts, evidence, required sections and technical content as they are.

For Codex, start a new chat in Worktree mode on the intended base branch and tell the factory to use that fresh worktree. The workflow verifies it is clean and matches the fetched remote base before creating the feature branch. Codex-managed worktrees may have their own lifecycle; retain the worktree while review follow-ups are needed. Depending on the sandbox, Git operations that write shared repository metadata, network access for fetch and GitHub, and dashboard access may still require permission. In other environments, configure the sibling worktree location as writable before starting the session if needed; that alone may not grant access to shared Git metadata.

In `/factory-full` and `/factory-balanced`, the agent commits the finished implementation before Check, because `code-review` only sees committed changes; it reviews everything since the run's base commit. Actionable Check findings send the run back to Build for a **Rework** with that mode's build approach; the fixes, including new files, are committed, and Check runs again over the commits since the previously reviewed one. A run gets at most two Reworks, and the dashboard shows the current one as "Rework 1/2". Findings alone never block a run, and the agent fixes them within the two Reworks. When findings remain after the second Rework, or one cannot be resolved, the run shows as blocked while the agent lists them and asks you whether to do one more Rework, finish with the findings listed in the pull request, or stop; the block clears when work resumes. `/factory-fast` has no Check and so no Rework.

### Choosing a mode

- **`/factory-full`**: risky, complex, or poorly understood work where you want every requirement questioned and every behavior built test-first. Slowest and most token-intensive.
- **`/factory-balanced`**: everyday features and fixes that deserve tests and a review but not a full interview or red-green cycles.
- **`/factory-fast`**: small, clear, low-risk changes such as documentation, configuration, or obvious fixes. No review stage.

Use `/factory` to assess the request and select a workflow. An explicit workflow choice bypasses selection. `/factory` previously ran Full directly; use `/factory-full` for that behavior. Existing runs recorded with the legacy `factory` mode remain readable as Full and are displayed as `factory-full`. Older binaries may not understand the new value, so use the normal update procedure to upgrade the command, dashboard server, and installed skills together.

The diagram shows the skills each mode runs, through Finish and the CI watch. Arrows between stages are labeled with the mode that takes them; `/factory-balanced` and `/factory-fast` run each Define skill only in the cases the table lists. Dotted lines show skills that another skill loads or applies, and the Rework loop from Check back to Build.

```mermaid
flowchart TB
    start([factory run start]) --> define

    subgraph define [Define]
        direction TB
        grill[grill-with-docs] --> spec[to-spec] --> tickets[to-tickets]
        grill -.-> grilling[grilling]
        grill -.-> domain[domain-modeling]
    end

    subgraph build [Build]
        impl[implement-spec] -.-> tdd[tdd]
        implbal[implement-spec-balanced] -.->|test-quality rules| tdd
        implfast[implement-spec-fast]
    end

    subgraph check [Check]
        review[code-review] -->|/factory-full always, /factory-balanced for security-sensitive diffs| security[security-review]
    end

    subgraph finish [Finish]
        pr[pr] --> pull([Pull request to the selected base branch])
        pull --> ci{CI green?}
        ci -->|no, fix in a new commit and push| pull
        ci -->|yes| done([Ready for review])
    end

    define -->|/factory-full| impl
    define -->|/factory-balanced| implbal
    define -->|/factory-fast| implfast
    impl --> review
    implbal --> review
    check -.->|Rework: findings, at most 2| build
    security --> pr
    review -->|/factory-balanced for other diffs| pr
    implfast -->|/factory-fast skips Check| pr
```

`factory install` also installs `codebase-design` and `triage`, which no workflow calls directly, and `setup-matt-pocock-skills`, which you run yourself (see below).

#### Token usage

The dashboard reads the token counts your agents write to their own session logs and shows them per run, stage, model and ticket. It counts tokens only, never money, and it keeps only counts, identifiers and the reasoning effort of each call: no conversation text, prompts, tool output or credentials are stored.

- `factory run stats <id>` prints the breakdown (stage, model and ticket, with the rework and work with no ticket in their own rows), the totals per model, and what the figures do not cover. `factory run stats --all` adds the totals per model over all runs and names the runs it leaves out.
- The dashboard shows the fresh tokens on a card and the breakdown in the run's detail.

**What the figures mean.** Every agent is normalized to the same four counts:

| Count | Meaning |
| --- | --- |
| Input | Input tokens that were not read from the cache. |
| Cache read | Input served from the cache. |
| Cache write | Tokens written to the cache. |
| Output | Output tokens, including reasoning tokens where the agent counts them in output. |

**Fresh tokens** are the headline figure: input excluding cache reads, plus cache writes and output. Cache reads are shown beside them, not inside, so large cached contexts do not hide the work. Reasoning is shown as "of which" and is never added to output twice. Codex and Copilot report cached tokens inside their input; the dashboard takes them out so all agents compare. A count an agent does not report is `n/a`, never zero.

**How a session is tied to a run.** Folder and time are never enough on their own. A session counts for a run when its id was recorded on the run, or its log holds a `factory run update`, `inspect` or `stats` command naming the run's id (the id elsewhere in a log, such as in a listing, does not count). `factory run start` and `factory run update` record Claude Code's session id automatically; for another agent, pass `--session <id>` when you know it. Within a session, each log entry goes to the stage, rework and ticket the run was in at that moment (the run keeps a small timeline of when those changed). Work that spans two runs at the same moment in one session is not divided by guessing: it is reported as not assigned. Subagent usage is included once, in the stage and model it ran under, and shown as a separate share.

**Coverage.** Usage is `tracked`, `partial` (with the reasons, such as a malformed or unreadable log, a stage with no data, or work not assigned) or `not tracked` (with the reason, such as no log naming the run, or a run from before usage tracking). Stored counts are kept when a vendor later deletes its logs.

**When it is collected.** By the dashboard, in the background, when a run changes stage, rework, ticket or session and when it finishes, and again about 15 and 60 seconds later, because agents write their logs a little after the work. It recomputes the whole run, so reading twice never counts twice, and a failure never delays or stops a run. Reading is bounded in time, in the number and size of the logs, and it does not follow links out of the log folders. There is no periodic reporting and no extra service.

**Opt out** with `factory dashboard --no-usage`, or by starting the dashboard with `FACTORY_USAGE=off`. Runs without counts then show `not tracked` and say why; counts already collected are kept.

**What has been validated.** Claude Code and Codex logs were inspected on a real machine, and all three formats are covered by fixture tests that need no agent installed. Copilot's `session.usage_record` events could not be checked against real logs (the local sessions held none), so Copilot support is fixture-tested only. Codex's `cache_write_input_tokens` was zero in every sample, so whether it is part of Codex's input is unvalidated; it is treated as included, like Copilot's. Agent log formats are not a stable interface: if one changes, the affected logs show as `partial` or `not tracked` instead of wrong numbers.

#### Starting a run with its workspace

`factory run begin "<title>"` creates a run and its workspace in one command. The workspace is a git worktree on a new branch, made from the pull request base branch. It replaces `factory run start`, the `git worktree add` and the `factory run update` that records them; `factory run start` still works as before.

```text
factory run begin "<title>" [--agent <codex|claude|copilot>] [--mode <factory-full|factory-balanced|factory-fast>]
                  [--base <branch>] [--branch <name>] [--json]
factory run begin --run <run-id> [--base <branch>] [--branch <name>] [--mode ...] [--agent ...] [--json]
```

- **Base.** `--base` names the branch. Without it the command uses the branch `refs/remotes/origin/HEAD` points to. It fetches that branch from `origin` and creates the worktree from `origin/<base>`; the recorded base commit is that commit. It does not read the project's documentation, so an agent passes the project's documented base itself.
- **Names.** `--branch` is used as given. Without it the branch is a kebab-case name from the title (ASCII letters and digits, at most 40 characters), with `-2`, `-3` and so on added until neither a branch nor a folder uses the name. The worktree is `<repo-parent>/<repo-name>.worktrees/<branch with / replaced by ->`, beside the main checkout, wherever the command runs. An explicit branch or folder that exists stops the command, and the retry command it prints leaves `--branch` out, so the retry picks a free name and never takes over what exists.
- **Recorded.** Mode, worktree, branch and base commit, the repository (as `run start` records it) and the agent session, with the stage set to define and the activity "Workspace ready". It prints the run ID first, then worktree, branch, base and base commit; `--json` prints the same.
- **Failures.** Once the run exists, a missing `origin`, an unknown base, a failed fetch, git that cannot write, or a name that is taken mark the run blocked with the reason, print the reason and the command to retry, and exit with status 1. Outside a repository, inside another `.git` folder (a submodule), or with a bad option, no run is created. Nothing is ever deleted, reset, checked out over or cleaned.
- **Retry.** `factory run begin --run <id>` resumes that run. The command to retry that a failure prints carries `--branch <name>` as soon as a name was chosen; a resume takes over an existing branch or worktree only when it is given that branch, and without one it picks a free name like a new run, so it never takes over another run's workspace. If nothing was made it does everything; a branch without a worktree gets its worktree; a clean worktree that was made but not recorded is recorded. Anything else (a folder that is not a worktree, a branch with commits of its own, a worktree with uncommitted changes) stops and says exactly what it found. A worktree that git lists but whose folder is gone is reported, and you run `git worktree prune` yourself. A run that is finished, or already has a workspace, is refused.
- **Dedicated worktrees.** When your agent environment hands you a fresh worktree, the Workspace procedure still uses `factory run start` and `factory run update` by hand; `begin` always makes its own worktree.

#### Opening a pull request

`factory pr open <run-id> --title "<title>" --body-file <path>` pushes the run's branch and opens its pull request, or finds the one that is open, and records its URL on the run. The agent still writes the title (one line, up to 256 characters) and the description (a file of up to 256 KiB). For GitHub it needs the GitHub CLI (`gh`), signed in with `gh auth login`. For Azure DevOps Services it needs the Azure CLI (`az`) and its `azure-devops` extension, authenticated with `az login` or `az devops login`. Git also needs credentials for the remote.

```text
factory pr open <run-id> --title <title> --body-file <path> [--base <branch>] [--json]
```

- **Base.** `--base` names the branch the pull request goes against; without it the command uses the branch `refs/remotes/origin/HEAD` points to.
- **Checks before anything is pushed.** The run has a worktree recorded and it exists; `gh` is installed; the worktree is on a branch (the one the run records, when it records one) whose name is safe to push; its tree is clean; the branch is not the base and has commits beyond `origin/<base>`; and `gh` is signed in and can see the repository, which is asked by looking for an open pull request before the push. A branch name that git could read as an option or a forced refspec, such as `+main`, is refused.
- **Push.** `git push -u origin refs/heads/<branch>:refs/heads/<branch>`, never forced. If origin rejects it because the branch has commits yours does not, the message says to merge them or the base into the branch with a normal merge and try again.
- **Retry.** It looks for an open pull request of the branch first, ignoring pull requests from forks. If there is one it is reused, so running the command again never makes a second pull request; new commits are still pushed to it. If another run of the command opens it between the look and the creation, the one that won is used. If the pull request opened but the server did not take the record, the message names the URL and a retry records it. For a run that is blocked or done the output says that the run was not updated and how to record the pull request.
- **Output.** `Pull request opened` or `Pull request already open`, the URL, the branch, the base and the pushed commit; `--json` prints the same fields (`pull_request`, `created`, `branch`, `base`, `commit`, `recorded`). It sets the run's pull request and the activity "Opened pull request" for a running run, keeping its stage, and leaves a run that is blocked or done alone. It never waits for CI: `factory pr wait` is the next step. Problems exit with status 1.

#### Keeping a pull request current

`factory pr sync <run-id>` brings the run's branch up to date with its base, the way the Finish procedure asks for. Conflict resolution and verification stay with the agent.

```text
factory pr sync <run-id> [--base <branch>] [--json]
```

| Outcome | Exit status | Meaning |
| --- | ---: | --- |
| `current` | 0 | The branch already contains `origin/<base>`. Commits of yours that are not pushed are reported and left alone. |
| `pushed` | 2 | The base was merged with a normal merge commit, it touched none of the change's own files, the branch had nothing that was not pushed yet, and the merge was pushed. Watch CI again with `factory pr wait`. |
| `needs-verification` | 3 | The base merged cleanly, but it touched files of the change (listed), or the branch has commits that are not pushed yet, or it was never pushed. The merge is not pushed: rerun the project's verification on the merged code, then push. |
| `conflict` | 4 | The merge conflicts and is left in progress, with the conflicting files listed. Resolve them, rerun the verification, commit and push. |

Exit status 1 is a problem with what to do.

- **Checks.** The run has a worktree recorded and it exists; it is on the recorded branch, whose name is safe to push; the tree is clean; no merge is in progress; and the branch is not the base. The base is `--base`, otherwise the branch `refs/remotes/origin/HEAD` points to, and it is fetched first.
- **Behind.** `git merge-base --is-ancestor origin/<base> HEAD`: exit 0 is current, 1 is behind, anything else is an error and never read as behind.
- **Overlap.** The change's own files (`git diff --no-renames --name-only <merge base> HEAD`, taken before the merge) against the base's files since the merge base. Without rename detection a renamed file counts under both names.
- **Merge.** `git merge --no-ff` with the message "Merge origin/<base> into <branch>": never a rebase, amend or fast-forward. It never merges the pull request.
- **Push.** `refs/heads/<branch>:refs/heads/<branch>`, never forced. If the push fails the merge commit stays, and running the command again pushes it, but only when HEAD is that merge exactly as git makes it on its own, on top of what the remote has, with no overlap; a merge that needs verification or a conflict resolved by hand is never pushed by the command. If someone else pushed to the branch, the command stops before merging and says so.
- **Run record.** A running run gets a milestone activity, such as "Merged develop into the branch", on its current stage and status; it is neither a Rework nor a CI fix.

Azure Repos pull request creation recognizes `dev.azure.com` HTTPS URLs, `ssh.dev.azure.com` v3 SSH URLs, and legacy `visualstudio.com` HTTPS URLs. The Azure DevOps CLI does not support Azure DevOps Server. Azure descriptions must be UTF-8 and at most 4,000 characters. Factory passes the organization, project, and repository explicitly to the CLI and checks for an existing pull request before pushing.

For a GitHub Enterprise host that cannot be identified from its remote, set `git config factory.prProvider github`. For a custom Azure Services remote, explicitly select the target with `git config factory.prProvider azure` and set `factory.azureOrganization`, `factory.azureProject`, and `factory.azureRepository` with `git config`. For an unrecognized custom remote, also set `factory.azureOrigin` to its exact `origin` URL, so stale configuration is rejected if the remote changes. For recognized Azure remotes, the configured destination must match `origin`. An unknown host without an override is refused.

The workflows' shared Pull request and CI procedure names the tool for each host. On any other host, such as GitLab, the agent opens the pull request and watches CI with that host's own tooling and records the URL on the run with `--pr`.

#### Watching CI

`factory pr wait <run-id>` watches the checks of the run's pull request and prints one CI outcome, so an agent does not poll with repeated model requests. It reads the pull request URL recorded on the run (`factory run update <id> finish running --pr <url>`) and uses the GitHub CLI (`gh`, signed in with `gh auth login`) for GitHub, or the Azure CLI with the `azure-devops` extension (signed in with `az login` or `az devops login`) for Azure DevOps Services.

| Outcome | Exit status | Meaning |
| --- | ---: | --- |
| `passed` | 0 | Every check of the current head commit passed or was skipped. |
| `failed` | 2 | A check failed. Each failed check is listed with its link. The first failed check of each Actions run, for at most three runs, also carries the last 40 lines (at most 4 KiB) of that run's failed log; a log that cannot be fetched leaves the name and link. A failure wins over a cancel. |
| `cancelled` | 3 | Checks were cancelled and none failed. |
| `timeout` | 4 | Checks were still running at `--timeout` (default 30 minutes, from 1 second to 24 hours, written as `90s`, `30m` or `2h`). |
| `head-changed` | 5 | The pull request got a new head commit while waiting. No check results are reported; run the command again. |
| `no-ci` | 6 | No check appeared within 90 seconds, or within `--timeout` when that is shorter. |
| `not-open` | 7 | The pull request is merged or closed. |

Exit status 1 is a problem with the tools or the run (no `gh` or `az`, not signed in, no network, no pull request recorded), with what to do. Checks are asked for every 15 seconds, and the progress goes to standard error. Right after a push, GitHub can still report the previous head commit for a few seconds. So when the run's worktree is on the commit its branch has on origin and the pull request shows another one, the command first gives GitHub up to 30 seconds to show the pushed commit, asking every 5 seconds, and then watches whatever head the pull request reports. That time counts toward `--timeout`. By default it waits until every check has ended; `--fail-fast` returns `failed` at the first failing check and lists the others as still running. `--json` prints the same data.

Next to the outcome it says whether the branch is up to date with its base, behind it, in conflict with it, or not yet known. GitHub is asked again up to five times. Azure reports a conflict when its merge calculation finds one and otherwise reports unknown, because a successful merge does not prove the branch is up to date; check it with `factory pr sync`. The command only reads: it never merges, pushes or updates the branch, and a `passed` outcome does not mark Finish done. While it waits, the run's activity is "Waiting for CI", and it is set to the outcome when the wait ends; a run that is blocked or done is left alone. Names, links and log lines come from the pull request's checks and are printed with control characters replaced. Ctrl-C ends the process; the activity text then stays until the next update of the run. The Finish procedure of the workflows uses this command, and falls back to watching by hand when the installed `factory` does not know it.

For Azure DevOps Services, record the canonical `https://dev.azure.com/<organization>/<project>/_git/<repository>/pullrequest/<id>` URL; a legacy `visualstudio.com` URL is refused with a request for the canonical one. Factory checks the source branch against the latest PR iteration and merge calculation before it accepts results, and checks it again after collecting them. It watches enabled, required policies and statuses of the current iteration:

- Approved policies and successful statuses pass.
- Rejected or broken policies and failed statuses fail. A rejected build policy is reported as cancelled only when its build run says it was cancelled.
- Queued, running, expired, or unknown states remain pending.
- A build policy counts only when its evaluation started at or after the current iteration; a missing or older start stays pending, even when Azure reports an approval. Other policies, such as minimum reviewers or linked work items, are re-evaluated by Azure, so their reported state counts as is.
- PR-wide statuses without an iteration cannot prove a result for the current head and remain pending, whatever state Azure reports for them. A newer status in the same context replaces an older one. So a pull request that has a PR-wide status, from any integration, never reaches `passed` or `failed`: the wait ends in `timeout`, or in `failed` with `--fail-fast` once another check fails.
- Optional policies are shown but do not block. Disabled and not-applicable policies do not count as CI.

Failure links point to the build when there is one, otherwise to the PR. Failed log excerpts are only available for GitHub Actions, and PRs from Azure forks are not supported.

#### Request analysis

`factory run requests <id>` shows how a run's model requests split over what they did, so you can see which steps cost the most requests. `factory run requests --all` pools many runs and shows how each category spreads over them. Add `--json` to either for the same figures as data. It reads the session logs when you call it and stores nothing. Of each request it keeps the time, the token counts and a category; no command text, tool output, prompt or path is kept or printed.

The counting rules:

- **Unit.** A model request is one response of the model. Claude Code writes a response once per content block, so a response that appears several times counts once, with the largest value of each token count.
- **Which run.** A session belongs to a run when its id was recorded on the run, or when a Bash command in it runs `factory run update <id>`. A run id in tool output, in a file or in a prompt does not count, and `inspect` and `stats` do not claim it. A command that only names the update, such as a `grep` or an `echo`, or text inside quotes, does not claim it either. The id may be a variable that an earlier command on the same line sets to a literal (`R=123 && factory run update $R ...`). A value that comes from a file or another command (`R=$(cat file)`) cannot be known here, so such a run is found through the session id recorded on the run, which Claude Code adds automatically. A request counts for a run when it falls inside the run's time, as the token usage does.
- **Overlapping runs.** A request that two runs working at the same time could both claim is in neither run. It is shown as not assigned.
- **Categories.** Each request has one category, taken from the first match in this order. The commands are read from the start of each part of a shell line, after env assignments and the words that only wrap a command (listed below).
  - Workspace: `git worktree` (other than `list`), `git clone`, `git symbolic-ref` and `factory run start`.
  - Commit: `git commit` and `git add`.
  - PR creation and push: `git push`, `gh pr create`, `gh pr edit`, `az repos pr create` and `az repos pr update`.
  - PR synchronization: `git fetch`, `git merge`, `git merge-base`, `git pull`, and `gh pr view` that asks for `mergeStateStatus`.
  - CI watch: `gh pr checks`, `gh run watch`, `gh run view`, `gh run list`, `gh api` calls on `actions/runs` or `check-runs`, `az pipelines runs`, `az repos pr policy list`, and `az devops invoke` on pull request statuses (`pullRequestStatuses`, `pullRequestIterationStatuses`). Other `az` commands, such as `az boards`, fall under reading, editing, analysis like any other tool call.
  - Verification: `cargo` with `test`, `fmt`, `clippy`, `build`, `check` or `nextest`; `rustfmt`; `pytest`; `npm`, `pnpm`, `yarn` and `bun` with `test`, `lint`, `build` (or `run` followed by `test`, `lint`, `build` or `check`); `go` with `test`, `vet` or `build`; `dotnet` with `test` or `build`; `make` with `test`, `check` or `lint`; and any `verify.sh`.
  - Continuation check: `factory run inspect`, `factory run check`, `git worktree list` and `git rev-parse --git-common-dir`.

  A request whose only command is `factory run update` is standalone reporting, any other tool call is reading, editing, analysis, and a request with no tool call is text only. Commands are split at `&&`, `||`, `;`, `|` and line ends outside quotes, the lines of a here-document are ignored, and only the first 32 words of a command are read. The words that only wrap a command are skipped: `sudo`, `time`, `command`, `exec`, `nohup`, `env` and `timeout` with their options (and the duration), the shell keywords `if`, `then`, `do`, `else`, `elif`, `while` and `until`, `!`, and an opening parenthesis or brace. Comments are ignored. A command substitution, `$(...)` or backticks, counts as one word of its command, and the commands inside it are classified too, because they run; a read-only helper in a substitution (`cat file`, `date`, `git rev-parse`) or after a pipe (`| tail -1`, `| grep`) does not make a report count as chained, but any other program there (`git apply`, `xargs`, `tee`, a script) is work of its own. The request that runs `factory run start` is written before the run exists, so a workspace request up to two minutes before the run's first mark counts for the run. The reader sees only the commands the agent types: what a script runs (`python3 - <<'EOF'`) is invisible, and syntax such as `case`, functions and aliases is not understood.
- **Reporting.** A `factory run update` chained to other commands does not decide the category: the request counts in the category of the other work and is also counted as chained reporting. That count sits beside the categories, so the category shares still add up to 100%. The table also gives the number of requests that fit more than one category, so you can see how much the order decides.
- **Tool output.** For each request, the bytes of text its tool calls returned, summed over its calls. Each category shows the total and the most one request got. Images and other non-text results are not counted.
- **Pooled view.** `--all` classes each run from its own data: completed (done, with fully tracked token usage), partial (done, but its usage is partial, missing or has stages with no data) or open (still running). It pools completed runs only; `--include-partial` adds the other two, and `--exclude <id>`, which you can repeat, drops a run, such as a development run. The factory cannot tell a development run from a production run. Runs it leaves out are listed with the reason, and so is a run for which no requests were found. For each category it shows the median, the 25th and 75th percentile (interpolated between the nearest runs), the minimum and the maximum of requests per run, counted over every pooled run so that a run without the category counts as zero. It also shows the pooled requests and their share, the share of fresh tokens, and the four token columns. The four token types are never added together; the fresh share is the one exception, and it uses the factory's usual definition of fresh tokens (input excluding cache reads, plus cache writes and output). The number of runs is in the heading, and a sample under ten runs is called small.
- **Tokens.** Input, cache read, cache write and output are separate columns and are not added together, except for the fresh share of the pooled view. `n/a` means the log did not report a count.
- **Subagents.** The requests of subagents come from the session's subagent logs. They are categorized by their own commands and shown on their own lines, marked `(subagents)`, and the total has an `of which subagents` line. A response that both a session and its subagent log hold counts once. A session with no subagent logs is reported as coordinator-only.
- **Codex.** Codex sessions are read from their rollout files. A response ends at its token record; the tool calls since the previous record belong to it, and their results, which come later, add their text to it. A command runs through a small JavaScript snippet, `tools.exec_command({cmd: "..."})`; the snippet is scanned for string literals and never run. A `cmd` that is a single plain literal (with its JavaScript escapes decoded) is classified like a Bash command. A `cmd` that is a variable, a concatenation or a template with `${...}` cannot be read, and neither can anything else Codex calls (`apply_patch`, `write_stdin`, web calls, the agent tools): each counts as a tool call that is not a command. A subagent rollout starts with a copy of its parent's history, which is not its own work, so unfinished calls are dropped when a turn starts. A Codex session claims a run through a recorded session id or an executed `factory run update <id>`, as for Claude Code; a mention of the id elsewhere in the log, such as in a file or a command's output, does not claim it.
- **Copilot.** Copilot sessions are counted from the usage reader, with the category `not categorized`. Their logs say nothing about tool output, so those columns show `n/a`, and the tool output totals cover Claude Code and Codex requests only. A Copilot session counts for a run by the usage reader's rule: its id was recorded on the run, or its log names the run.

### Model routing

Model routing lets you say which model and effort each kind of task should use, independently of the workflow mode. The factory records your choices on the run, and the role mapped to `check` sets the model and effort of the review agents of `factory-full` and `factory-balanced`. The `build` and `finish` tasks are not routed yet and run on the session's model. Without a policy file, runs behave as before.

Write one policy file, `model-routing.json`, in the factory's config folder: `$XDG_CONFIG_HOME/tds-factory/` (or `~/.config/tds-factory/` when unset) on Linux, and `%APPDATA%\tds-factory\` on Windows. A relative `XDG_CONFIG_HOME` is ignored. One file can hold independent settings for Claude Code and Codex, each under its agent's name in `agents`:

```json
{
  "enabled": true,
  "agents": {
    "claude": {
      "roles": {
        "strong": { "model": "opus", "effort": "high" },
        "coding": { "model": "sonnet", "effort": "medium" },
        "routine": { "model": "haiku", "effort": "low" }
      },
      "tasks": { "check": "strong" }
    },
    "codex": {
      "roles": {
        "strong": { "model": "gpt-6-astra", "effort": "high" }
      },
      "tasks": { "check": "strong" }
    }
  }
}
```

`factory doctor --create-model-routing` writes a starter like this for you, with an entry for each agent (or only one with `--agent claude` or `--agent codex`); see [Checking readiness](#checking-readiness). It never overwrites a file that is already there.

- Each entry under `agents` is `claude` or `codex` and has `roles` and `tasks`, and optionally `enabled`. `roles` defines up to three roles (`strong`, `coding`, `routine`), each with a `model` and an optional `effort`. `tasks` maps `build`, `check` and `finish` to one of those roles; a task you leave out is not routed. `enabled` defaults to `true`; set it to `false` at the top to switch routing off for every agent, or inside an entry to switch off that agent only, without deleting anything.
- The original single-agent form still works and is never rewritten for you: `agent` (`claude` or `codex`), `roles` and `tasks` at the top of the file, instead of `agents`. Use one form or the other; a file that has both is refused.
- Models and efforts are plain text of up to 64 letters, digits and `. _ : / -`. The factory keeps no list of valid values, so a new model works without a factory release, and checking that your agent accepts a value is up to you.
- `factory run start "title" --agent <claude|codex|copilot>` names the agent a run is in; the workflow skills pass it for you. The factory reads the file once, records only that agent's policy on the run, and never uses another agent's:
  - With the `agents` form the option is required, because the file cannot tell which agent is asking. Without it the start stops and names the agents the file configures. An agent with no entry, or a switched-off entry, is not routed; the first also prints a notice that says which entry to add.
  - With the single-agent form the option is optional, so existing invocations keep working. Without it, or with the policy's own agent, the policy is recorded; for the other agent nothing is recorded and a notice says so.
  - `--agent copilot` records nothing and prints the Copilot notice, whatever the file holds.
  - Continuing a run keeps the recorded selection, so editing the file affects only runs you start afterwards.
- A file that cannot be read, is too large, is not valid JSON, has an unknown field or an unknown agent, repeats a key (an agent, a role or a task), has an empty `agents`, has an `agents.copilot` entry, or maps a task to a role that is not defined stops `factory run start` with the file path and the problem, naming the entry (`agents.codex`) when the mistake is in one. Every enabled entry is checked each time, so a typo in Codex's entry also stops a Claude Code run until it is fixed. A policy with `enabled: false`, and a switched-off entry, are not checked.
- To give one task a different role on a running run, use `factory run update <id> <stage> <status> --model-routing check=coding` (repeat the option for more tasks). The options you give replace all earlier overrides, and `--clear-model-routing` removes them. The role must be one the run's policy defines, and a run started without a policy has nothing to override. The policy's own mapping is not changed, and the override is kept when the run is continued.
- When a run's selection maps `check` to a role, the review agents that Check starts use that role's model and effort:
  - The workflow tells the `code-review` skill which model and effort to use, on every Check pass including the one after a Rework. The coordinating session keeps its own model.
  - On Claude Code the agents are fresh subagents started with the Agent tool's `model` and `effort`, which accepts only the aliases `opus`, `sonnet`, `haiku` and `fable`. On Codex they use its subagent model and effort settings.
  - A security review that the coordinator runs itself stays on the coordinator's model. The stage summary says what was applied or why routing was not.
  - Without a selection, for a run with no role for `check`, or on Copilot CLI, Check works as before. Build still runs in the coordinator without subagents, and `factory-fast` has no Check to route.
- `factory run inspect <id>` shows each task's model and effort under "Model routing", marks overridden tasks, and `--json` carries the selection in `model_routing`. Under each routed task it also lists the models the run's [token usage](#token-usage) shows for the stage of the same name, with the reasoning effort the logs recorded and `+subagents` where a subagent used that model:
  - An alias such as `sonnet` and an observed id such as `claude-sonnet-5-5` differ in form, so both are shown. The request is met when it is a whole part of an observed id used by any session in that stage, subagents included, so Opus review agents satisfy an `opus` request even if the coordinator ran Sonnet.
  - When usage is tracked and the requested model never appears, the line says `MISMATCH`. When usage is only partial it says the model was not observed, because it may be missing, and every observed line notes partial coverage.
  - A stage with no usage rows, or a run whose usage is not tracked, says so.
  - When an effort was requested, it is compared with the efforts recorded for the models that match the request. A different effort is a `MISMATCH` (or "not observed" when usage is partial), and a log that records none says `effort not recorded`. Claude Code and Codex record the effort of each call; Copilot's usage records do not.
  - The request counts as met when any one of the recorded efforts is the requested one. When the coordinator and the review agents run the same model at different efforts, their efforts land in the same row, so the agents' effort cannot be told apart from the coordinator's. A row keeps at most eight distinct efforts.
  - Runs recorded before this feature, and runs without a selection, show no Model routing section.

Copilot CLI is not supported. It chooses a subagent's model from its own settings per agent name, and nothing documents a way for the main agent to pick a model when it starts a subagent. Supporting it would need extra agent files and behavior that Copilot does not promise, so every stage keeps the session's model. A single-agent policy with `"agent": "copilot"`, or a run started with `--agent copilot`, records no selection and prints a notice.

### Checking readiness

`factory doctor` reports whether the current repository and your machine are ready for the factory, without changing anything, installing anything or starting the dashboard, unless you ask it to create the model-routing policy file (see below). It checks:

- the project: a Git repository, an `origin` remote, origin's default branch (`git remote set-head origin --auto` sets it when unknown), the setup output of `/setup-matt-pocock-skills` (`docs/agents/issue-tracker.md` and the `## Agent skills` section), which `/factory-full` always needs and the lighter workflows only when they read or publish specs or tickets, and the verification commands listed as bullets under `## Commands` in `AGENTS.md` or `CLAUDE.md`. It looks for each command's tool but never runs it.
- each agent (Codex, Claude Code, Copilot): its command is on `PATH`, and every file of every skill `factory install` bundles is installed in the folder `factory install` would use (honoring `CODEX_HOME`, `CLAUDE_CONFIG_DIR` and `COPILOT_HOME`).
- the pull request host, in a "Provider" section after the agents. `host` says whether origin is on GitHub or Azure Repos, worked out the way `factory pr open` does it (including `factory.prProvider`). An unknown host, or an override that does not match origin, is a warning that says to open pull requests with the host's own tooling; without an origin the section is `skipped`. For GitHub, doctor looks for `gh` on `PATH` and runs `gh auth status`. For Azure Repos, it looks for `az`, runs `az extension show --name azure-devops`, and counts you as signed in when `AZURE_DEVOPS_EXT_PAT` is set or `az account show` succeeds. It only checks that the variable is set, never reads or prints the token, and discards what these commands print. A `not-verified` line says what it did not check: whether the token is valid, project permissions and branch policies. A missing tool or sign-in is a warning, so this section never changes whether the factory is ready.

- the [model routing](#model-routing) policy, only when a `model-routing.json` exists and is switched on. A "Model routing" section follows the provider section. Doctor asks the agent for its `--version` there, using the program's absolute path from `PATH`; that is the only agent command it runs, and it calls no model.
  - `policy`: the file loads. A file that stops `factory run start` is a `fail`, which includes a role without a model and a mistake in any enabled entry, named as `agents.<agent>`. With the `agents` form, the line says which agents the file configures, and each agent's checks that follow are tagged with it, as `host [claude]` in the text and an `agent` field in `--json`.
  - `factory doctor` checks every configured entry. `--agent claude` or `--agent codex` checks only that agent's entry: an agent with no entry, or a switched-off entry, is `skipped` with the reason, and so is a single-agent file written for the other agent. With no `--agent`, an agent without an entry is left out.
  - `host`: for Claude Code, the command is on `PATH` and is version 2.1.292 or later, the first with the Agent tool's `effort` option. For Codex, the command and its version are shown, but the factory does not know which version first supports a model and effort per subagent, so that is `skipped`.
  - `model` and `effort`: for Claude Code, each role a task uses must name `opus`, `sonnet`, `haiku` or `fable`, the only models the Agent tool accepts, and an effort of `low`, `medium`, `high`, `xhigh` or `max`. A role without an effort is flagged, because the model's default applies and differs between models. Which levels a particular model offers is not checked. For Codex the factory keeps no list, so both are `skipped`.
  - `overrides`: for Claude Code, a warning when `CLAUDE_CODE_SUBAGENT_MODEL_FORCE=1` together with `CLAUDE_CODE_SUBAGENT_MODEL`, or `CLAUDE_CODE_EFFORT_LEVEL`, is set in the environment, your Claude Code `settings.json`, or the project's `.claude/settings.json` or `.claude/settings.local.json` (their `env` entries), because they replace the policy's model or effort. Only string values are read, and enterprise-managed settings are not, so an override set there is missed. A settings file that cannot be read is `skipped`.
  - A policy for Copilot CLI shows one `pass` line: model routing is not supported there and every stage uses the session's model.

Each line is `pass`, `missing` (something to supply), `fail` (something broken), `warning` (a risk to read about that does not stop the factory) or `skipped` (not checked, with the reason), with a fix where there is one. Model routing problems other than a policy file that cannot be used are warnings, because model routing is optional and without it a stage uses the session's model. The exit status is 0 when the project checks pass and at least one agent is usable, and the model-routing policy, if there is one, loads. Use `--agent codex|claude|copilot` to check, and require, one agent, and `--json` for structured output.

Doctor writes one file, and only when you ask: a starter model-routing policy. `factory doctor --create-model-routing` creates `model-routing.json` in the config folder in the `agents` form, with an entry for Claude Code and one for Codex, each with the three roles and only the `check` task routed to `strong`. Add `--agent claude` or `--agent codex` for an entry for that agent only. Doctor then runs its checks, so the new file is verified straight away. Giving the option is the confirmation: there is no prompt. The file is switched on, so the next run's review agents use it; edit the models and efforts, or set `"enabled": false`. Doctor never overwrites anything already at that path, and never writes through a symbolic link there. If it cannot write the file, it stops with the reason and does not run its checks. The option is refused with `--agent copilot`, because Copilot CLI is not supported. With `--json`, the note about the file goes to stderr and stdout stays JSON.

### First use in a repository

Run `/setup-matt-pocock-skills` once in each repository before the first workflow run. It records the repository's issue tracker (GitHub, GitLab, Azure Boards, local Markdown files or another tracker you describe), triage labels and domain-doc layout in `docs/agents/` and adds an `## Agent skills` section to `AGENTS.md` or `CLAUDE.md`. Run it in your normal checkout; there is no need to commit it or push it to a protected branch first. Each workflow run starts in a fresh worktree and carries the uncommitted setup output over from the checkout it was started in, so the output lands in that run's pull request. `/factory-full` stops at the start of Define when `docs/agents/issue-tracker.md` exists neither in the worktree nor in the original checkout. `/factory-balanced` and `/factory-fast` need it only when a run reads or publishes a spec or tickets in the tracker; a clear, bounded request planned in the conversation runs without it. A run whose plan exists only in the conversation saves its plan and acceptance criteria in a file outside the pull request (in the worktree's git directory, or beside the worktree) and records it with `--spec`, so another session can continue from `factory run inspect` without the original chat.

The [Rust skill](skills/rust/SKILL.md) and [project instructions](AGENTS.md) guide Rust development in this repository.

## Development

Day-to-day work targets `develop`; releases merge `develop` into `master`. See [the release guide](docs/releasing.md) for the release and branch-protection setup.

Run `scripts/verify.sh` before finishing a change and opening a pull request. It runs the same checks as CI, in the same order, and stops at the first failure; it needs `cargo`, `rustfmt` and `clippy` (`rustup component add rustfmt clippy`). The individual commands are:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```
