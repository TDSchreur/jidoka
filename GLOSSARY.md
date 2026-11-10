# Glossary

**Run**: One piece of work carried through the Jidoka flow.

**Stage**: The current part of a run: Define, Build, Check, or Finish.

**Needs you**: A run that is blocked and waiting for the user. The dashboard lists it in the andon strip, marks its row on the line amber, sorts it first and counts it in the page title.

**Andon strip**: The top of the dashboard. It says that nothing needs you, or turns amber and lists every run that needs you, with its project and status reason. Named after the signal board above a factory line.

**The line**: The dashboard's list of active runs (every run that is not done), one row each, with a rail of the four Stages that shows where the run is.

**Shipped**: The dashboard's name for a finished run, one with the status `done`. Shipped runs are listed below the line, grouped by day. The CLI, the API and `runs.json` keep the status `done`.

**Pull request state**: Whether a run's pull request is open, a draft, merged or closed. The dashboard server reads it from GitHub with `gh` or from Azure Repos with `az`, keeps it on the run and reads it again about once a minute until it is merged. When it cannot be read, it is unknown, with the reason; it is never guessed.

**Rework**: A return from Check to Build to fix actionable findings, followed by another Check. `jidoka-full` and `jidoka-balanced` allow at most three per run; after the second, findings that change only wording or documentation are fixed without asking and the user decides only on findings that change behaviour; after the third, the user decides whether to do another Rework, finish with the findings listed in the pull request, or stop. Findings alone never block a run; the run is `blocked` only while it waits for that decision.

**Workflow mode**: The workflow skill that carries a run: `jidoka-full` (quality), `jidoka-balanced`, or `jidoka-fast` (speed). Modes differ in how much Define, Build, and Check do.

**Model routing**: Choosing the model and effort each Stage's agents run on, independently of the workflow mode. The coordinating session keeps its own model; a routed Stage hands its work to agents started on the Stage's model, while conversation with the user stays in the coordinating session. It is not the same as workflow selection, which `/jidoka` does. A run records its model-routing selection when it starts.

**Model routing policy**: The user-owned file that describes model routing: for each agent (Claude Code, Codex, Copilot CLI), a model and optional effort per Stage. A Stage left out keeps the session's model. A run starts in one agent and records only that agent's part. Jidoka reads the file once, when a run starts.

**Run context**: What a new agent session needs to continue a run: the repository, worktree, branch and base commit it started from, the workflow mode, a reference to the spec, the ordered tickets with their progress, the pull request, a short summary per Stage and the status reason. It holds references and summaries only, never transcripts or secrets, and is changed only through `jidoka run update`. Anything a run never recorded is shown as unknown, not guessed.

**Repository**: The canonical main checkout of the project a run belongs to. Every linked worktree of a checkout has the same repository, so the dashboard groups their runs in one row.

**Status reason**: Why a run is blocked, in the run's own words. It is the blocking reason a run is blocked with.

**Continuing a run**: Starting a new session on an existing run. The session reads `jidoka run inspect <id>`, checks the recorded repository, worktree, branch, base commit and Git state, reports any difference to the user and never resets, cleans or discards work to make them match.

**Fresh tokens**: The headline token figure of a run: input excluding cache reads, plus cache writes and output. Cache reads are shown beside it, and reasoning is part of output and not added again.

**Session token**: The random value in the dashboard page that the page sends as `X-Session-Token` to delete runs. Any local process can read it through `GET /`, so it does not authorise POST.

**CLI token**: The random value the dashboard writes on each start to `session-token` (`session-token-<port>` for a non-default port) in its data folder, readable only by the user. If the dashboard cannot write it, the dashboard exits. A POST without the right value gets 401. The `jidoka` CLI sends it as `X-Jidoka-Token` on every POST. It is never part of a response, and differs from the Session token.

**Token usage**: The tokens a run used, read from the agents' session logs and kept as counts per stage, model and ticket (plus the reasoning efforts the logs recorded), with the rework and work with no ticket in their own buckets. It is `tracked`, `partial` or `not tracked`, with the reason; unknown usage is never shown as zero.

**Estimated cost**: The cost of a run's token usage in USD, computed when it is shown as a token-based estimate at API list prices, not subscription charges, from Jidoka's built-in price table (per million tokens, Claude and OpenAI models) for each row, model, stage and run. A model without a price is `unknown`, never zero; a total that includes one is partial (`>= $X`). It is not stored.

**Pause**: A moment the agent's Claude Code log shows work stopped: a usage limit or a context compaction. The dashboard keeps its kind and time, blocks a running run with the matching reason until later activity appears, and shows it on the run's timeline. A stretch of over 10 minutes with no activity that no pause explains is an idle gap. Codex and Copilot logs are not checked: Copilot's `events.jsonl` writes no usage-limit or compaction events.

**Model request**: One response from the model, counted once however many times its agent's log repeats it. It is the unit of the request analysis; user turns and individual tool calls are different units.

**Request category**: The kind of work a model request did, taken from the commands in its tool calls: workspace, commit, PR creation and push, PR synchronization, CI watch, verification, continuation check, reading/editing/analysis, or text only when it called no tool. Each request has exactly one category. Standalone reporting is a request whose only command updates the run; a run update chained onto another command counts in that command's category and is also counted as chained reporting, which is a count and not a category.

**CI outcome**: The single result of watching a pull request's checks for its current head commit: `passed`, `failed`, `cancelled`, `timeout`, `head-changed`, `no-ci` or `not-open`. Whether the branch is behind its base is reported next to the outcome and is not part of it.

**Workspace**: A run's own git worktree and feature branch, created from the pull request base branch, which the run records as its worktree, branch and base commit. `jidoka run begin` creates it.

**Run cleanup**: Removing a run's Workspace once its pull request is merged: `jidoka run cleanup <id>` removes the worktree and the local feature branch, never discarding uncommitted changes or commits that were not in the merged pull request, and clears them from the run context.

**Ticket report**: A short, agent-reported skill or activity associated with a ticket in a run. Reports may be incomplete and are not independent proof of the work.

**Run report**: A short, agent-reported skill or activity associated with a run rather than a particular ticket. Reports may be incomplete and are not independent proof of the work.

**Reader**: The dashboard's view of a run's local spec or ticket, rendered from the Markdown file the run recorded. It shows only documents the run recorded, and says so when the file is no longer there.
