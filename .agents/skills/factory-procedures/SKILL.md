---
name: factory-procedures
description: Shared operating procedures for the factory-full, factory-balanced and factory-fast workflows: run reporting, continuing a run, workspace, tracker setup, model routing, writing, and pull request and CI. Read by those workflows; do not run it on its own.
---

# Factory procedures

The `factory-full`, `factory-balanced` and `factory-fast` workflows read this file for the procedures they share. Each workflow keeps its own planning, testing, review and verification policy, and its instructions take precedence over this file where they differ. Do not start a run from this skill.

## Run reporting

Report progress to the factory dashboard with the `factory` CLI from the run's worktree (see Workspace). Update only at the points below; do not send periodic updates.

- **Start:** before Define, run `factory run start "<short task title>" --agent <agent>` and keep the printed run ID. `<agent>` is the coding agent you are running in: `claude` for Claude Code, `codex` for Codex, `copilot` for Copilot CLI. The factory uses it to pick that agent's model-routing policy, and says on standard error when it applies none; keep that for the Check summary. If the command stops with a message about the policy, tell the user what it says and stop. If the server is not running, start `factory dashboard` as a background process, then retry once. If a different or outdated server occupies the port, or the CLI rejects an option (including `--agent`), report that it needs to be stopped or upgraded.
- **Stage:** at the start of each later stage the workflow has, run `factory run update <id> <stage> running --activity "<what the stage is doing>"`.
- **Milestone:** after a meaningful milestone, such as tickets created or checks passing, run `factory run update <id> <stage> running --activity "<short phrase>"`. Keep the activity to one short line without secrets.
- **Ticket:** when work on a ticket with an HTTP(S) URL starts, add `--current-ticket "<url>"` to that update. When no ticket is in progress anymore, add `--clear-current-ticket`. Leave it unset for local Markdown tickets.
- **Blocked:** when work cannot continue without outside input, run `factory run update <id> <stage> blocked --blocking-reason "<what is needed>"`. When work resumes, run `factory run update <id> <stage> running --clear-blocking-reason` (a Rework adds `--rework <n>` to that update). Waiting for the user's decision on Check findings is such a case; the workflows with Check name the reason.
- **Context:** record what a new session needs to continue the run, with `factory run update <id> <stage> <status>` options, only through the CLI and never by editing the factory's files. Each of these is a reference or a short summary, never a transcript, a secret or a token. Options left out keep their value; `--clear-<name>` removes one.
  - Once the worktree exists: `--mode <the workflow's name> --worktree <absolute path> --branch <branch> --base-commit <commit the worktree started from>`.
  - Once a spec or tickets exist: `--spec <url or path>`, and one `--ticket "<reference> <title>"` per ticket in order (list them all each time; their statuses are kept). When a ticket starts, `--ticket-status <reference>=in-progress`; when it is finished, `--ticket-status <reference>=done`. A run with no tickets records none.
  - When a stage ends: `--summary <define|build|check|finish>=<one or two sentences>`. Summaries of earlier stages are kept.
  - When the pull request exists: `--pr <url>`.
  - A reason you block a run with `--blocking-reason` is the run's status reason; make it say what is needed.
- **Done:** once the pull request is open and its CI is green (or the project has no CI), run `factory run update <id> finish done --activity "Opened <pull request URL>" --clear-current-ticket`.

Options that are left out keep their current value. If the server or an update fails, report the failure rather than claiming progress is tracked.

## Continuing a run

When the user asks you to continue an existing run, or gives you a run ID, read its context with `factory run inspect <id>` (add `--json` for structured output) before changing anything, and check the recorded state against reality. Fields shown as unknown were never recorded: do not guess them.

1. The recorded repository exists, and `git -C <repository> rev-parse --git-common-dir` agrees with the worktree's.
2. The recorded worktree exists and `git worktree list --porcelain` lists it.
3. In that worktree, `git branch --show-current` is the recorded branch.
4. The recorded base commit is an ancestor of `HEAD` (`git merge-base --is-ancestor <base commit> HEAD`).
5. `git status --porcelain` shows no unexpected changes, and any commits not yet pushed are listed.

Report every discrepancy and every unknown to the user, with what you found. Never reset, clean, check out over, delete or discard anything to make the state match, and do not start new work while something differs. If everything matches, read the recorded spec or saved plan from its reference (it holds the acceptance criteria), then continue in the recorded worktree from the recorded stage, at the first ticket that is not done, and record new context as you go. If something differs, ask the user how to proceed.

## Workspace

Each run must have its own fresh git worktree. Never switch branches or modify files in the user's main checkout; the dedicated run worktree is where changes belong. Determine the project's documented PR base branch, otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`, and fetch it with `git fetch origin <base>`.

If the user explicitly starts this run in a fresh, dedicated worktree (for example, one provisioned by their agent environment), it may be the run's worktree. Verify that it is a linked worktree, is clean, has not been used for another run, and its `HEAD` equals `origin/<base>` after the fetch. Use a new feature branch there; if a branch is already checked out, only use it if it was created for this run and is not the base branch. Do not reset, clean, or discard changes to make a worktree eligible. If any condition is not met, create a new worktree instead, or stop and explain the constraint if the environment cannot do so.

Otherwise, choose a short feature branch name and run `git worktree add -b <branch> <repo-parent>/<repo-name>.worktrees/<branch-with-slashes-as-dashes> origin/<base>`. If the branch or path already exists, choose a new name. If the worktree cannot be created, mark the run blocked with the reason and stop.

Run every later stage from the run's worktree: Define files, Build, checks, `factory run update` commands, commit, push, and the pull request. Respect the environment's permissions: a writable worktree does not necessarily grant write access to shared Git metadata or network access. Report any required permission rather than assuming it is available.

## Tracker setup

Follow this only when the workflow says the tracker is needed: always in `/factory-full`, and in the lighter workflows only when the run reads or publishes a spec or tickets in the tracker. A clear, bounded request planned in the conversation does not need it, so never block such a run on it. When it is needed, check that `docs/agents/issue-tracker.md` exists in the run's worktree. If it is missing, look first in the checkout where the run started if it is distinct from the run worktree, then in the main checkout identified by `git worktree list --porcelain` (the first worktree, if it is not bare). If it exists in either checkout (setup was run but not committed to the base branch), copy that checkout's `docs/agents/` directory into the worktree and bring its `## Agent skills` section into the worktree's `AGENTS.md` or `CLAUDE.md` (whichever the source checkout edited), so the pull request includes the setup output; do not modify the source checkout. If it is absent from all available checkouts, or the main repository is bare and no setup source is available, mark the run blocked with `--blocking-reason "Run /setup-matt-pocock-skills first"`, tell the user to run `/setup-matt-pocock-skills` once in the original checkout and then start the run again (the new worktree will pick up its output; this run's worktree can be removed), and stop. Do not guess a tracker.

## Saving the plan

When a run's plan and acceptance criteria exist only in the conversation, save them before Build so another session can continue without the original chat. Skip this when the spec is already a tracker issue or a file in the repository, and record that reference instead.

1. Write a concise Markdown file with the task, the plan and the acceptance criteria to `factory-plan.md` in the run worktree's git directory (`git rev-parse --absolute-git-dir`). If that is not writable, write `<worktree path>.factory-plan.md` beside the worktree instead and remove it together with the worktree. The file stays out of the pull request.
2. Record it with `factory run update <id> <stage> <status> --spec <absolute path>`. This is the same context option as a tracker spec, so no tracker is involved.
3. When the plan changes, rewrite the file. Never save secrets or a transcript.

## Model routing

A run can carry a model-routing selection: the model and effort each kind of task should use. The policy file can describe several agents, and the run records only the entry of the agent named with `--agent` when it started, never another agent's. The `check` task is the one that uses the selection: it covers the review agents that Check starts. The coordinating session, which is you, keeps its own model, and so does every step you run yourself.

1. Read the selection with `factory run inspect <id> --json`. `model_routing` holds `agent` (`claude` or `codex`), `roles` (each with a `model` and an optional `effort`), `tasks` (task to role) and `overrides` (task to role, which win over `tasks`). The role for the `check` task is `overrides.check`, otherwise `tasks.check`, and its model and effort are in `roles[role]`. A run without `model_routing`, or without a role for `check`, is not routed. On Copilot CLI, or when the policy has no entry for your agent, no selection is recorded, so nothing is routed. If `factory doctor` warned that the policy cannot apply here, treat the task as not routed.
2. Use it only for agents you start for that task, and only when `agent` is the agent you are running in. Every Check pass uses it, including the one after a Rework. When you invoke `code-review`, tell it the model and effort, because its Standards and Spec sub-agents are the agents to route. On Claude Code, the Agent tool's `model` accepts only the aliases `opus`, `sonnet`, `haiku` and `fable`, so pass the recorded model only if it is one of them; set `effort` to the recorded effort, and start a fresh subagent, never a fork, because a fork ignores `model`. On Codex, set the model and effort of the agent you start through the subagent settings Codex documents. If the recorded model is not one the host takes, or the host refuses a setting, such as a Claude Code that rejects `effort`, start the agent with neither the model nor the effort, so an effort is never paired with a model it was not chosen for.
3. Run `security-review` on the `check` selection only if you start it as a separate agent the same way, on every pass. Run inline in your own session, it uses your model: do not claim a switch for it.
4. Say what happened in the stage summary: "Check agents ran on opus, effort high", or "model routing not applied: <reason>" when there is no selection, `check` has no role, `factory doctor` warned that the policy cannot apply, the recorded agent is not the one you run in, the model is not one the host takes, or the host refused a setting.

## Writing

Use the `humanizer` skill for human-facing prose you write or edit in a run: specs, tickets, plans, documentation, changelog entries, review summaries, commit messages, and pull request descriptions. Apply it to new or changed text only; leave unrelated prose alone.

- Use Humanizer's embedded mode for text you compose (a spec, ticket, summary or description) and its file mode for edits to an existing document. Put only the finished text in the result: no intermediate drafts, no critique of the draft.
- Stay inside the task's scope and output format. Keep required headings and sections, evidence and its labels, findings, qualifications, notices, domain terms, numbers, and technical content (code, commands, paths, metadata, data, link targets) exactly as the task needs them. Humanizer's formatting advice never overrides these, and it never adds a detail the source does not have.
- Follow the project's own writing conventions where they differ from Humanizer's style advice.

## Pull request and CI

Once the project's verification has passed on the final commit, push the branch and create a pull request targeting the base branch chosen in Workspace. Then watch the pull request's CI: follow every check until all have completed, noticing failures as well as success, and keep the run running with `--activity "Waiting for CI"`. When a check fails, read its log, fix the cause in a new commit on the same branch (never amend or force-push), rerun the project's verification on the changed code, push, note it with a milestone update, and watch the new run; repeat until CI is green. CI fixes are not Reworks. A failure you cannot fix, such as an infrastructure problem, a missing permission, or a flaky external dependency, blocks the run with that reason. When the project has no CI, skip the watch and say so. Never merge the pull request; that is the user's decision. Report the pull request as ready for review only when CI is green. Keep the worktree for review follow-ups. Report its link, the worktree path (removable with `git worktree remove <path>` after merge), the checks performed, and the final CI result. If a required tool, permission, or repository configuration is unavailable, report what remains, mark the run blocked with that as the reason, and stop.
