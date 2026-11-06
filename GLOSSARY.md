# Glossary

**Run**: One piece of work carried through the factory flow.

**Stage**: The current part of a run: Define, Build, Check, or Finish.

**Needs you**: A run that is blocked and waiting for the user. The dashboard marks its card, sorts it first and counts it in the page title.

**Rework**: A return from Check to Build to fix actionable findings, followed by another Check. `factory-full` and `factory-balanced` allow at most two per run; after that, the user decides whether to do another Rework, finish with the findings listed in the pull request, or stop. Findings alone never block a run; the run is `blocked` only while it waits for that decision.

**Workflow mode**: The workflow skill that carries a run: `factory-full` (quality), `factory-balanced`, or `factory-fast` (speed). Modes differ in how much Define, Build, and Check do.

**Model routing**: Choosing the model and effort a kind of task runs on, independently of the workflow mode. It is not the same as workflow selection, which `/factory` does. A run records its model-routing selection when it starts. Copilot CLI is not supported: every stage there keeps the session's model.

**Model routing policy**: The user-owned file that describes model routing: for each agent (Claude Code, Codex), the roles (`strong`, `coding`, `routine`) with a model and effort each, and which role handles build, check and finish. A run starts in one agent and records only that agent's part. The factory reads the file once, when a run starts.

**Run context**: What a new agent session needs to continue a run: the repository, worktree, branch and base commit it started from, the workflow mode, a reference to the spec, the ordered tickets with their progress, the pull request, a short summary per Stage and the status reason. It holds references and summaries only, never transcripts or secrets, and is changed only through `factory run update`. Anything a run never recorded is shown as unknown, not guessed.

**Repository**: The canonical main checkout of the project a run belongs to. Every linked worktree of a checkout has the same repository, so the dashboard groups their runs in one row.

**Status reason**: Why a run is blocked, in the run's own words. It is the blocking reason a run is blocked with.

**Continuing a run**: Starting a new session on an existing run. The session reads `factory run inspect <id>`, checks the recorded repository, worktree, branch, base commit and Git state, reports any difference to the user and never resets, cleans or discards work to make them match.

**Fresh tokens**: The headline token figure of a run: input excluding cache reads, plus cache writes and output. Cache reads are shown beside it, and reasoning is part of output and not added again.

**Token usage**: The tokens a run used, read from the agents' session logs and kept as counts per stage, model and ticket (plus the reasoning efforts the logs recorded), with the rework and work with no ticket in their own buckets. It is `tracked`, `partial` or `not tracked`, with the reason; unknown usage is never shown as zero.

**Model request**: One response from the model, counted once however many times its agent's log repeats it. It is the unit of the request analysis; user turns and individual tool calls are different units.

**Request category**: The kind of work a model request did, taken from the commands in its tool calls: workspace, commit, PR creation and push, PR synchronization, CI watch, verification, continuation check, reading/editing/analysis, or text only when it called no tool. Each request has exactly one category. Standalone reporting is a request whose only command updates the run; a run update chained onto another command counts in that command's category and is also counted as chained reporting, which is a count and not a category.

**CI outcome**: The single result of watching a pull request's checks for its current head commit: `passed`, `failed`, `cancelled`, `timeout`, `head-changed`, `no-ci` or `not-open`. Whether the branch is behind its base is reported next to the outcome and is not part of it.
