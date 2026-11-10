# Agent flow eval (Copilot)

This eval checks what GitHub Copilot CLI does next in four situations Jidoka procedures cover: failed verification, resumed work that still matches the plan, resumed work that differs from it, and a pull request that already exists. Run it after installing the skills for Copilot, and before and after changing `jidoka-procedures` or its references, to see whether a choice shifted. It only measures Copilot; Claude Code and Codex are not covered.

It calls a paid model once per case in each repetition, so it is not part of CI or `scripts/verify.sh`.

## Validation status

A case with `"validated": true` has been run five times against Copilot, with the same accepted action every time. All four current cases passed that check; `verification-fails` always chose `fix`, so its other accepted action, `block`, has never been seen. A case with `"validated": false` has not been checked yet: the runner prints `UNVALIDATED` next to it and counts it in the summary, so treat its result as a hint. New cases start as `false`; set `true` after running the case several times with Copilot and checking its answers.

## Running it

Install the skills first (`jidoka install --agent copilot`, or `scripts/update-local.sh`), then run:

```sh
evals/agent-flow-copilot/run.sh                 # every case, 3 times each
evals/agent-flow-copilot/run.sh --runs 1        # a quick check
evals/agent-flow-copilot/run.sh --case pr-already-created --runs 5
evals/agent-flow-copilot/run.sh --model gpt-5
```

It needs `copilot`, `jq`, and GNU `timeout` (on macOS, from coreutils). `--skills-dir` points at a different skills directory; the default is `$COPILOT_HOME/skills`, otherwise `~/.copilot/skills`.

The output has one line per case with its passes, its validation status, the actions it saw and the accepted actions, then the overall pass rate. A `?` means no `ACTION:` line was found, for example because `copilot` failed or timed out after ten minutes; the script then says which, and prints the end of what `copilot` said.

| Exit status | Meaning |
| --- | --- |
| 0 | Every answer chose an accepted action. |
| 3 | At least one answer was wrong. |
| 1 | A usage error or a missing tool. |

## How it works

For each case the runner asks Copilot to read `jidoka-procedures/SKILL.md` and its references, describes the scenario, and lists the possible actions by key. Copilot must end with `ACTION: <key>`. The session runs in an empty temporary directory with only `view`, `grep` and `glob` available, so no command runs and no file changes. The scenario states the outcome of commands such as `jidoka run check` instead of running them.

## Adding a case

Add an object to `cases.json`:

```json
{
  "id": "short-name",
  "validated": false,
  "scenario": "The situation, including the output of any command.",
  "question": "What is your very next action?",
  "actions": { "right": "The right action.", "wrong": "A tempting wrong action." },
  "accepted": ["right"]
}
```

Keep `"validated": false` until you have checked the case against Copilot as described above.
