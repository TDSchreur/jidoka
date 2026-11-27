# What to look for

Signals point at where to dig; none is a finding on its own. For each, the question that separates a Jidoka problem from a legitimate run is given.

## Jidoka bugs

**State and reporting**

- `inspect` shows a stage, status or gate that contradicts what happened (a finished run still `running`, a run in Check with no Build, gates passed for a commit that is not the reviewed one). Compare with the events and timeline in `inspect --json`.
- `check` reports `unknown` or `not-checked` for the worktree, branch or base commit although the run used a worktree. Was it recorded at the start (`jidoka run begin`) or skipped by the agent? Only the first is a Jidoka bug. `jidoka run cleanup` clears the worktree and branch on purpose, so a finished and cleaned-up run shows `never recorded`; that is not a finding.
- A finished run that still carries a blocking reason, or an `inspect` that disagrees with its own events.
- `advance --skip-ci` was needed: the CI check could not be read. Read the recorded reason; a host error that should not have blocked CI is a bug.
- `advance --force` was needed. Read the recorded reason: a gate that could not be met, or a gate that wrongly refused, is a bug; a skipped step is not.
- A command printed an error, a panic or a message that is wrong or unhelpful, or refused a valid update. Search the session for `jidoka` commands that returned a non-zero status.

**Attention and blocking**

- `events` holds `blocked` ("Agent stopped; it may need you") followed by `resumed` within seconds or minutes with no user action in between: a likely false positive. Check what the agent was doing at that time (background work, a subagent, a long tool call).
- A run left blocked after the agent continued, or never blocked while it waited for the user.
- A block that matches the documentation (a user-input request or a finished turn) is correct even when the wording is vague. Confirm in the agent's session log what the agent did at the time of the `blocked` event before calling it false.
- Pauses for usage limits or compaction that the dashboard did not show, or showed wrongly.

**Usage tracking**

- `Tokens` is `partial` or `not tracked` although the run's session log exists. Read the reasons; list them.
- Rows with `no ticket`, `not assigned` or a stage with no data in a run that had tickets and stages.
- A model with `unknown` cost although `jidoka run stats` documents a price for it, or a cost that is clearly wrong against the token counts.
- Model routing: `inspect` shows what was requested and what was observed for each stage. A mismatch (the stage ran on another model or effort than requested) that is not explained by the line next to it is a routing bug or a skipped instruction.

**Skills and stage instructions**

- The agent did something that the installed instructions told it to do and that was wrong, impossible or contradicts another instruction (`jidoka run next <id>` prints the current stage's instructions). Quote both instructions.
- A command named in an instruction does not exist, has other options, or behaves differently.

**Environment**

- `doctor` failures or warnings that look wrong for the setup (for example a tracker or base branch reported unavailable when it is configured).
- Windows, Linux or macOS specific failures: note the operating system.

## Token waste caused by Jidoka

Establish a baseline first: the user's pooled runs from `requests --all` (median, 25th and 75th percentile, per category), or, if the sample is small, say so and compare with what the run's own stages cost.

- **Rework**: `rework` above zero, or several entries in `check_history`. Read why. Findings that were real are the process working. Rework caused by a gate or instruction that was ambiguous, by a check that ran on the wrong commit, or by the same finding returning is a finding about Jidoka.
- **Process requests**: a high share in `continuation check`, `PR synchronization`, `CI watch`, `workspace` or `standalone reporting`, compared with the baseline. These are pure overhead. A repeated poll of the same state is the clearest case.
- **Chained reporting** counts that are high show the agent spending a request per report instead of batching.
- **Oversized tool output**: the `Largest` column or one request with tens of kilobytes of output. Which command produced it? If a Jidoka command's own output is the cause (for example a listing that grows with the run), that is a finding; if it was the agent reading a big file, it is not.
- **Repeated work across stages**: the same files or the same instruction document read again in each stage, a stage redoing the previous one, or a new session re-reading everything because the previous one was lost. Cache reads are shown beside fresh tokens; a large and growing `cache_read` with little output is context carried around.
- **Subagents**: a large subagent share in a stage that only needed a small check. Compare fresh tokens with and without subagents, and look at what the subagents did.
- **Model and effort**: a stage run on a more expensive model or higher effort than the routing policy asks for, or the whole run staying on the most expensive one.
- **Idle and pause cost**: a long gap followed by a session that restarted from scratch, or compaction repeated several times in one stage.

Report tokens as numbers: fresh tokens per stage and in total, the requests per category, the estimated cost, and the baseline median and range. A statement like "too many tokens" without these is not usable.

## Is it Jidoka, or the task?

Ask before you keep a finding:

1. Would a run of the same task cost the same without Jidoka's process? If yes, it is the task.
2. Did the user or the agent choose the behaviour freely (an extra review, a manual retry)? If yes, it is not a defect.
3. Can you point at a Jidoka command, document or skill text that caused or allowed it? If you cannot, the finding is probably not about Jidoka.

If the answer to the third is a skill or stage instruction, name the file and quote the lines; the maintainers change those.
