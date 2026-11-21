---
name: jidoka-run-report
description: "Analyze a finished or stuck Jidoka run for Jidoka bugs and avoidable token usage, and file a detailed GitHub issue in the Jidoka repository so the maintainers can fix it. Use whenever the user wants to review, audit, debug or report on a Jidoka run (a run id), asks why a run was expensive, slow, stuck or wrongly blocked, says Jidoka misbehaved, or wants to report a Jidoka bug or token problem, even when they do not say 'issue' or 'report'."
---

# Jidoka run report

Codex only: when requesting escalation for any Jidoka command eligible for reusable approval, set the tool’s `prefix_rule` to the specific command verb, for example `["jidoka", "run", "check"]`. Keep run IDs and other changing arguments in the command itself.

Analyze one Jidoka run for two kinds of problems and, when you find real ones, file them as GitHub issues in the Jidoka repository (<https://github.com/TDSchreur/jidoka>). Maintainers can only act on a report that proves the problem and lets them reproduce it, so the evidence matters more than the verdict.

The two kinds, and nothing else:

- **Jidoka bug**: Jidoka itself behaved wrongly: a command, the dashboard, usage tracking, attention detection, gates, stages, installed skills or stage instructions. A bug in the user's own project is not a Jidoka bug, even when the run was about fixing it.
- **Token waste caused by Jidoka**: the run spent clearly more tokens than it needed because of how Jidoka works: a loop, repeated or oversized output, a skill or stage instruction that makes the agent redo work, unnecessary rework. A run that was expensive because the task was big is not a finding.

Style, naming, slow tests in the user's project and feature wishes are out of scope. Say so briefly if the user asks for them.

## 1. Find the run

Use the run id the user gave. Without one, ask for it: the dashboard and `jidoka run inspect` show it, and it is a long number. Do not guess between runs.

## 2. Collect the evidence

Everything below is read-only. Save the output to files in a temporary folder (your scratchpad or the system temp folder) so you can quote from it later instead of rerunning it. Run each command that applies; a command that fails is evidence too, so keep its message.

```text
jidoka --version
jidoka run inspect <id> --json
jidoka run stats <id>
jidoka run requests <id> --json
jidoka run requests --all --include-partial
jidoka run check <id> --json
```

Run `jidoka doctor --json` only for a finding about setup or the environment; it describes the project in the current folder, not the run. `--include-partial` keeps this run in the baseline when its own token usage is partial, which would otherwise leave it out; say so in the report when you rely on it.

Also read the run's plan file if `inspect` names one, and the agent's session log only as far as a finding needs (see the next step). `inspect --json` lists the run's session ids under `sessions` and `model_routing.agent` names the agent. The run may have been made with a different agent than the one you are running in, so find each log by its id in the owning agent's folder (the folder moves when the agent's config variable is set; on Windows `~` is the user profile folder):

| Agent | Log, found by session id | Folder variable |
| --- | --- | --- |
| Claude Code | `~/.claude/projects/<project>/<id>.jsonl`, plus a `subagents` folder next to it | `CLAUDE_CONFIG_DIR` |
| Codex | `~/.codex/sessions/<year>/<month>/<day>/rollout-...<id>.jsonl` | `CODEX_HOME` |
| Copilot | `~/.copilot/session-state/<id>/events.jsonl` | `COPILOT_HOME` |

Search the folder for a file or folder name containing the id; never search file contents of the whole tree. A log is the only way to see what the agent was doing at a given time, such as when a block began. If you are inside the run's own session you already have that history. Read the entries around that time, not the whole file.

`requests --all` pools the user's other finished runs and is the baseline for "is this run unusually expensive" in requests, stage by stage and per category; it has no fresh-token column per run, so compare fresh tokens with the pooled token columns or with the other stages of this run, and say which you used. It may report that the sample is small or that no runs qualify; then say the baseline is weak instead of inventing one.

## 3. Look for problems

Read [references/signals.md](references/signals.md) for what to look for and how to tell a Jidoka problem from an expensive-but-legitimate run. Work through bugs first, then tokens.

For every candidate, find out why before you believe it. Numbers alone prove little: a run with three reworks may have had three real review findings. Open the evidence behind the number: the gate that failed and its evidence text, the events with their stages and times, the request category that stands out, the commands of the agent in the session log. Where this conversation is the run's own session, you already have that history.

Keep a candidate only when you can state all of these in one sentence each:

1. What Jidoka did.
2. What it should have done, citing the documentation, the command's own help or the stage instructions (`jidoka run next <id>`) as the source of "should".
3. What it cost or broke: tokens, requests, time, a wrong state, a stalled run.

A symptom you can prove with the run's data and the code can still have an unproven cause; file it as "symptom confirmed, cause unconfirmed" and say what you ruled out. Do not reproduce a problem by creating runs or changing the user's real Jidoka data; the commands write to the same store the dashboard uses. Describe what the existing run shows instead.

Drop a candidate you cannot support. Mark one you suspect but cannot prove as unconfirmed rather than dropping it, so the maintainers can decide, but never present a guess as a fact.

## 4. Decide what to file

File one issue per root cause, not one per symptom and not one big list: separate issues can be fixed, linked and closed on their own. Two symptoms of the same cause (for example a false block and the tokens spent resuming from it) are one issue.

A suspected security vulnerability is never filed as a public issue. Point the user to the private security advisory form named in the repository's security policy and stop for that finding.

If you found nothing, say so plainly, with what you checked. The same goes for behaviour that matches the documentation but misled the user (a correct block with unhelpful wording): it is not a bug. You may offer it as an enhancement idea and file it only if the user wants that. An honest empty report is a good result; do not stretch a weak signal into an issue.

## 5. Check for duplicates

Before drafting, search the open and closed issues for each finding with two or three distinct keywords, then again with the wording of the symptom as a user would write it (for example "Agent stopped" or "false blocked"). Closed issues count: a fixed problem that returns is a regression and worth reporting as one, a duplicate of an open or wontfix one is not.



```text
gh issue list --repo TDSchreur/jidoka --state all --search "<keywords>" --limit 10
```

Read a promising hit with `gh issue view <number> --repo TDSchreur/jidoka` (body and comments) before deciding whether it covers the problem.

If the same problem is already reported, do not file a new one. Offer to add a comment with this run's evidence instead, as that is often the most useful thing a report can do. Only do so when the user agrees.

## 6. Draft the issue

Use the structure in [references/issue-template.md](references/issue-template.md). It follows the repository's bug report form so triage works the same. The details make a report useful, so include them: exact versions, the agent and model, the stage and times, the commands and their output, the numbers with the baseline they were compared with, and steps to reproduce that someone else can follow. Cut anything that does not support the finding.

The issue is public, so you must remove private material before it leaves the machine:

- Run ids, issue and pull request numbers, timestamps and token counts are identifiers, not content, and may stay.
- Never include prompts, conversation text, source code, secrets, tokens, email addresses or user names.
- Replace private repository names, organisation names, absolute paths and branch names with neutral placeholders (`<repo>`, `<home>/project`).
- A run title or ticket title can reveal the user's product. Describe the run generically ("a run that added a feature to a web service") unless the title is harmless.
- Quote output only as far as it shows the problem, and trim it. Jidoka stores counts and identifiers, not text, so most evidence is safe, but check each excerpt.

## 7. Confirm, then file

Show the user each drafted issue in full (title, body and where it will go) and wait for a clear yes. Filing is public, cannot be fully undone and posts under the user's GitHub account, so approval to analyze is not approval to file. Let the user edit or drop findings first.

On a yes, write the body to a file and create the issue without a shell heredoc, which does not work in every shell. Do not pass labels: people without triage rights cannot set them, and the command fails; maintainers label during triage.

```text
gh issue create --repo TDSchreur/jidoka --title "<title>" --body-file <body-file>
```

Check that `gh` is installed and signed in first (`gh auth status`). If it is not, save the body to a file, tell the user its full path, and give them <https://github.com/TDSchreur/jidoka/issues/new/choose> to paste it into.

Finish by listing each issue's URL (or the saved file) and the findings you dropped or left unconfirmed, with the reason.
