# Issue body

One issue per root cause. Fill every section; write "not available" rather than leaving one out. Keep it scannable: the first screen should let a maintainer judge the problem, and the long evidence goes in collapsed blocks.

## Title

A specific, searchable sentence about Jidoka's behaviour, not the user's task. Start with the area when it helps.

- Good: `Dashboard marks a Codex run blocked while a subagent is still working`
- Good: `Check stage re-reads the full stage instructions on every rework (+40k fresh tokens)`
- Bad: `Run was slow`, `Token usage too high`

## Body

````markdown
**Type:** bug | token usage

## What happened

<What Jidoka did, what it should have done and the source of that expectation (documentation page, command help, stage instruction), and what it cost or broke. Three to six sentences.>

## Impact

<For a bug: what went wrong for the user (wrong state, stalled run, lost work).
For token usage: the numbers, in a table.>

| Measure | This run | Baseline (median, range of N runs) |
| --- | --- | --- |
| Fresh tokens, total | | |
| Fresh tokens in the affected stage | | |
| Requests in the affected category | | |
| Estimated cost | | |

<Remove the table for a bug that has no token impact. Say when the baseline is a small sample or missing.>

## Steps to reproduce

1. <The setup: Jidoka version, agent, mode, what kind of project and task.>
2. <The commands or actions, in order.>
3. <What you then see.>

<If you did not reproduce it (usually the case: do not create runs in the user's real data), write "Not reproduced; this is what the run's data shows" and list the facts and the commands to see them on any similar run. Mark what you know and what you assume.>

## Evidence

<The two or three facts that prove the problem, each with its source: the field of `jidoka run inspect --json`, a row of `jidoka run requests`, a log line with its time. Quote only the relevant lines.>

<details>
<summary>Full output</summary>

```text
<trimmed output of the commands that support the finding>
```

</details>

## Suspected cause

<Optional. The command, document or skill text that you think is responsible, with file and quote. Say "not investigated" when you did not look.>

## Environment

- Jidoka version: <`jidoka --version`>
- Operating system: <Linux x86-64 | Windows x86-64 | Other>
- Coding agent and model: <Claude Code | Codex | GitHub Copilot, model and effort per stage; also the agent you ran this analysis in, when different>
- Workflow mode: <jidoka-full | jidoka-balanced | jidoka-fast>
- Run: <run id> (stage <stage>, status <status>)
- Confidence: <confirmed | unconfirmed, and why>

---
Found by the `jidoka-run-report` skill.
````

## Checks before showing the draft

- Every number has a source and, for tokens, a baseline or the statement that none exists.
- Steps could be followed by someone who has not seen the run.
- No prompts, source code, secrets, personal data, private names or absolute paths remain.
- The claim says what is known and what is assumed.
