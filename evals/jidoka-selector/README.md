# Jidoka selector eval

This eval checks which workflow the installed `jidoka` skill picks for a request: Full, Balanced, Fast, or no new run. Run it before and after changing the selector or the text every run loads (`jidoka`, the three workflows, `jidoka-procedures`). That way you can see whether the change shifted any choice.

It calls a paid model once per case in each repetition, so it is not part of CI or `scripts/verify.sh`. One repetition of all cases takes a minute or two and cost between $0.50 and $1.20 on the Claude Code default model in the first trials.

## Running it

Install the skills you want to test first (`jidoka install --agent claude`, or `scripts/update-local.sh`), then run:

```sh
evals/jidoka-selector/run.sh                 # every case, 3 times each
evals/jidoka-selector/run.sh --runs 1        # a quick check
evals/jidoka-selector/run.sh --case serde-bump --runs 5
evals/jidoka-selector/run.sh --model sonnet
```

It needs `claude`, `jq`, `jidoka`, `git`, and GNU `timeout` (on macOS, from coreutils), plus `awk` and `comm`. `--skills-dir` points at a different skills directory; the default is `$CLAUDE_CONFIG_DIR/skills`, otherwise `~/.claude/skills`.

The output has one line per case with its passes, the modes it saw and the accepted modes, then the overall pass rate and the cost reported by Claude Code. A `?` means no `MODE:` line was found, for example because `claude` failed or timed out after ten minutes. The script then says which of the two happened, or that the answer had no `MODE:` line, and prints the end of what `claude` said.

| Exit status | Meaning |
| --- | --- |
| 0 | Every answer chose an accepted mode. |
| 3 | At least one answer was wrong. |
| 2 | A jidoka run or a worktree appeared during the eval. |
| 1 | A usage error or a missing tool. |

## How it works

`run.sh` copies `fixture/` into a temporary directory, commits it as a new repository, and runs each request there:

```text
claude -p "/jidoka Which jidoka workflow would this request get if it were run for real, and why?
  Only answer the question: do not start a run ... Request: <request> ...
  MODE: none only when the request itself should not start a new run, such as continuing one ..."
  --add-dir <skills dir>              print mode reads nothing outside the working directory otherwise
  --permission-mode dontAsk           refuse every tool call that is not allowed below
  --tools Read Grep Glob Bash         no Write or Edit in the session at all
  --allowedTools Read Grep Glob "Bash(jidoka run inspect:*)" "Bash(git status:*)" "Bash(git log:*)"
  --disallowedTools Write Edit "Bash(jidoka run begin:*)" "Bash(jidoka pr:*)" "Bash(git worktree:*)" ...
```

The prompt asks for analysis only, which the selector's first rule respects, but asks which workflow the request would get if it were run, so the analysis-only question itself is not a reason to answer `none`. The tools can only read, and the deny list wins over any broader allow rule in your own Claude Code settings, so a run can't start, a worktree can't be created, and no file can change. After the last case, the script compares the run IDs in `jidoka run stats --all` with the ones from before, and checks that the temporary repository still has a single worktree. If anything appeared, it names it and exits with status 2.

When an answer has no `MODE:` line, the script prints the end of `claude`'s error output and of the answer, so you can tell a failed call from an answer in the wrong format. It reads `MODE: fast`, `**MODE:** fast` and ``MODE: `fast` `` alike.

The fixture is a small repository: a README with the typo, a guide with old wiki links, a `Cargo.toml` with `serde` 1.0.190, and an `AGENTS.md` that says most code named in a request is hypothetical. The selector judges each request as described instead of finding no code.

## Adding a case

Add an object to `cases.json`:

```json
{ "id": "short-name", "request": "What the user asks for.", "accepted": ["balanced"] }
```

`accepted` lists every mode that counts as right: `full`, `balanced`, `fast`, or `none` when the selector should not start a new run (for example when continuing one). List two modes when both are reasonable, as for the `serde` bump. If the request needs a file to make sense, add it under `fixture/`.
