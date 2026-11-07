# Factory selector eval

This eval checks which workflow the installed `factory` skill picks for a request: Full, Balanced, Fast, or no new run. Run it before and after changing the selector or the text every run loads (`factory`, the three workflows, `factory-procedures`). That way you can see whether the change shifted any choice.

It calls a paid model about ten times per repetition, so it is not part of CI or `scripts/verify.sh`. One repetition of all ten cases takes a minute or two and costs about $1.20 on the Claude Code default model.

## Running it

Install the skills you want to test first (`factory install --agent claude`, or `scripts/update-local.sh`), then run:

```sh
evals/factory-selector/run.sh                 # every case, 3 times each
evals/factory-selector/run.sh --runs 1        # a quick check
evals/factory-selector/run.sh --case serde-bump --runs 5
evals/factory-selector/run.sh --model sonnet
```

It needs `claude`, `jq`, `factory` and `git`. `--skills-dir` points at a different skills directory; the default is `$CLAUDE_CONFIG_DIR/skills`, otherwise `~/.claude/skills`.

The output has one line per case with its passes, the modes it saw and the accepted modes, then the overall pass rate and the cost reported by Claude Code. A `?` means no `MODE:` line was found, for example because `claude` failed or timed out after ten minutes.

| Exit status | Meaning |
| --- | --- |
| 0 | Every answer chose an accepted mode. |
| 3 | At least one answer was wrong. |
| 2 | A factory run or a worktree appeared during the eval. |
| 1 | A usage error or a missing tool. |

## How it works

`run.sh` copies `fixture/` into a temporary directory, commits it as a new repository, and runs each request there:

```text
claude -p "/factory Which factory workflow fits this request, and why? Do not start a run ... Request: <request> ... MODE: ..."
  --add-dir <skills dir>        print mode reads nothing outside the working directory otherwise
  --allowedTools Read Grep Glob Skill "Bash(factory run inspect:*)" "Bash(git status:*)" "Bash(git log:*)"
```

The prompt asks for analysis only, which the selector's first rule respects. The allowed tools can read, but they can't start a run, create a worktree or change a file. After the last case the script compares the run IDs in `factory run stats --all` with the ones from before, and checks that the temporary repository still has a single worktree. If anything appeared, it names it and exits with status 2.

The fixture is a small repository: a README with the typo, a guide with old wiki links, a `Cargo.toml` with `serde` 1.0.190, and an `AGENTS.md` that says most code named in a request is hypothetical. The selector judges each request as described instead of finding no code.

## Adding a case

Add an object to `cases.json`:

```json
{ "id": "short-name", "request": "What the user asks for.", "accepted": ["balanced"] }
```

`accepted` lists every mode that counts as right: `full`, `balanced`, `fast`, or `none` when the selector should not start a new run (for example when continuing one). List two modes when both are reasonable, as for the `serde` bump. If the request needs a file to make sense, add it under `fixture/`.
