---
name: jidoka-work-through-plan
description: Work through a big multi-step plan file unattended, one step at a time, each step implemented by a fresh agent that carries it through to a merged pull request. Reads the plan, finds the next open step, hands it to a fresh agent, verifies the merge, updates the plan file, and repeats until every step is done. Use whenever the user gives a path to a plan, roadmap or checklist note and asks to work through it, execute it, run all its steps, "do the plan", or continue where the plan left off, even if they never say "skill" or "loop".
---

# Work through a plan

A plan file is a list of steps. Each step is implemented by its own **fresh agent** so that no step inherits the context, assumptions or mistakes of the one before. This session is only the **orchestrator**: it reads the plan, hands out one step at a time, checks the outcome, and records it in the file. Keep the orchestrator's own context small and do not implement anything yourself.

The plan file is the single source of truth. Re-read it at the start of every iteration instead of trusting memory, because the user or an earlier iteration may have edited it.

## Input

The path to the plan file. If the user did not give one, ask once. If the user names a single step, run only that step.

## Plan format

Plans usually look like this; adapt to small variations:

- `## N. Title` headings, one per step, in recommended order.
- A checklist of observable outcomes (`- [ ]` / `- [x]`), the last item usually "PR merged ...".
- A status line such as `Run / PR / verification: —`.
- A fenced block holding the **prompt** for that step.
- Optional `Prerequisite: Step N merged.` lines, and notes on ordering and independence in the intro.

## The loop

1. **Read** the whole plan file, including the intro, which holds rules that apply to every step.
2. **Find open steps.** A step is done only when all its boxes are ticked and its status line names a merged pull request. A step with a recorded run or pull request but unfinished boxes is *in progress*: resume it instead of starting over. A step marked blocked is not open; see "When something goes wrong".
3. **Stop if nothing is open.** Report a short summary (step, pull request, result per step) and end.
4. **Pick the next step**: the lowest-numbered open step whose prerequisites are merged. Check prerequisites against reality (the host's pull request status, or the base branch's history), not only the file. If open steps remain but none is eligible, stop and say what blocks them.
5. **Hand the step to a fresh agent** using the agent's sub-agent mechanism, in the foreground and never by continuing an earlier step's agent. If the agent has no sub-agents, start a new non-interactive session instead. Its prompt contains, in this order: the step's prompt block copied verbatim, the plan's shared intro rules, and the unattended contract below.
6. **Verify the outcome yourself** from the agent's report. Confirm with the host's CLI that the pull request is merged into the base branch, and that the base branch contains it. Do not accept the agent's word for the merge.
7. **Update the plan file** with minimal edits: tick the boxes the agent verified with evidence, fill the status line (run, pull request link, one-line verification result), and tick the final "merged" box only after step 6 confirmed it and any cleanup or refresh the plan requires has finished. Note deviations briefly.
8. **Go back to 1.**

## Unattended contract for the step agent

Put this in every spawned agent's prompt, adapted to the plan, because nobody is there to answer questions:

- Make the best decision when something is unclear, record the reasoning in the pull request description, and continue. Do not stop to ask for confirmation.
- Work the step through to the end: implement, run the project's required verification, open the pull request, wait for required checks, fix failures, and merge once checks pass. The user authorized merging for this plan, only into the base branch the plan names (when it names none, the project's documented pull request base, else the remote's default branch).
- Do the project's post-merge steps (cleanup, reinstall or refresh) when its instructions or the plan require them.
- Follow the project's own instructions, including any rule about which workflow or skill to start with.
- Stay within the step's scope. When a prerequisite is missing, or the step needs a decision that cannot be made safely (for example a contract change that needs a human-owned decision record), stop and report **BLOCKED** with the reason instead of guessing.
- Finish with a compact report of about 15 lines: run or branch, pull request URL, merged yes/no, verification commands and results, which plan outcomes are verified, and deviations from the plan.

## When something goes wrong

Unattended does not mean unbounded; the loop must not spin or silently skip work.

- **Agent reports BLOCKED, fails, or the pull request is not merged:** record the state in the plan (run or branch, pull request link, one-line reason, step marked `Blocked:`), stop the whole loop and report to the user. Do not retry in a loop and do not jump to a later step, since later steps may depend on it.
- **One resume is fine:** if the agent died or ran out of context mid-step but left a run or pull request behind, start one fresh agent told to resume it. If that also fails, treat the step as blocked.
- **Plan file not writable:** continue, keep a running status list, and print the exact lines to record at the end.
- Never force-push over others' work, bypass failing required checks, or merge into a branch the plan did not name.

## Final report

When the loop stops (all done, blocked, or no eligible step), tell the user in a few lines: steps completed this session with their pull requests, the step that stopped the loop and why, and what is left.
