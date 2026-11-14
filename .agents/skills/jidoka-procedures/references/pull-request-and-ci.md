# Pull request and CI

Jidoka's pull request commands work with two hosts, and each needs its own command line tool, signed in: `gh` for GitHub, and `az` with the `azure-devops` extension for Azure Repos (signed in with `az login`, or with a personal access token in `AZURE_DEVOPS_EXT_PAT`). They work out the host from `origin`. `jidoka doctor` shows the host it found, and whether the tool and a sign-in are there. For any other host, or when the host is not configured, open the pull request and watch its CI with that host's own tooling, then record the URL on the run with `--pr <url>`. `jidoka pr sync` needs only git, so it works on any host; the rest of this file still applies, with the host's own tooling in place of `jidoka pr open` and `jidoka pr wait`. Watch every check there until all have completed, noticing failures as well as success, with `--activity "Waiting for CI"` on the running run.

## Opening the pull request

Once the project's verification has passed on the final commit, push the branch and create a pull request targeting the base branch chosen in Workspace. Write the title and the description first (the description in a file), then run `jidoka pr open <run-id> --title "<title>" --body-file <path> --base <base>`. It checks that the run's worktree is clean and on the recorded branch, that the branch is not the base and has commits beyond it, and that the host's tool is signed in; then it pushes the branch (never forced), opens the pull request, or finds the one that is already open for the branch, records its URL on the run, and prints it (`--json` prints data). If it stops with a message, fix what it says and run it again: a retry never makes a second pull request.

## Watching CI

Watch the pull request's CI with `jidoka pr wait <run-id>`. It follows the checks of the pull request's current head commit until all of them have ended, sets the activity of a running run to "Waiting for CI" itself, and prints one outcome and a matching exit status:

- `passed` (0): every check passed or was skipped.
- `failed` (2): a check failed. The output lists each failed check with its link, and the end of the failed log of up to three runs.
- `cancelled` (3): checks were cancelled and none failed.
- `timeout` (4): checks were still running at the end of `--timeout` (default 30 minutes).
- `head-changed` (5): the pull request got a new head commit while waiting, so the results say nothing about it; run the command again.
- `no-ci` (6): no check appeared within 90 seconds (or within `--timeout` when that is shorter), so the project has no CI.
- `not-open` (7): the pull request was merged or closed.

Exit status 1 is a problem with the tools or the run (no `gh` or `az`, not signed in, no network, no pull request recorded); its message says what to do. Next to the outcome the command says whether the branch is up to date, behind its base or in conflict with it. It never merges, pushes or updates the branch, and a `passed` outcome does not mark Finish done. On `not-open` it also brings the run in line with the pull request, as `jidoka run sync <run-id>` does: a merged pull request clears the activity and marks Finish done, a closed one clears the activity and blocks the run with the reason "PR closed without merge". The dashboard does the same on its own within about a minute, and its "Check PR" button does it at once. Add `--fail-fast` to get a `failed` outcome as soon as one check fails.

## Keeping the branch current

Keep the pull request current with its base branch. A base branch that requires up-to-date branches refuses a stale pull request even when CI is green, and another pull request may land while yours is open. Check before you push, and again each time a CI watch ends or a check fails. Run `jidoka pr sync <run-id> --base <base>`. It fetches the base, checks that the worktree is clean and on the recorded branch, and reports one outcome:

- `current` (0): the branch already contains the base.
- `pushed` (2): it merged the base with a normal merge commit, the base touched none of the change's own files, and it pushed the merge; watch CI again with `jidoka pr wait`.
- `needs-verification` (3): it merged the base cleanly, but the base touched files of the change, which it lists; rerun the project's verification on the merged code, then push.
- `conflict` (4): the merge is left in progress with the conflicting files listed; resolve them, rerun the verification on the merged code, commit and push.

Exit status 1 is a problem it explains, such as a dirty tree or a failed fetch. It also refuses (exit 1, nothing merged or pushed) when the pull request is already over: the run's stored pull request state is merged or closed, or the branch has commits of its own and all of them are already in the base. For a merged pull request run `jidoka run cleanup <run-id>` instead. It never rebases, amends, force-pushes, resets or aborts, it never pushes commits of yours that are not pushed yet (it reports them and does not push the merge either), a renamed file counts as touched, and if its own push of a clean merge failed, running it again pushes that merge; it never pushes a merge that needs verification or a conflict you resolved. A base update is neither a Rework nor a CI fix; `jidoka pr sync` notes it on the run. A conflict you cannot resolve with confidence blocks the run with that reason.

Once the pull request exists, the base state that `jidoka pr wait` prints is a second view. On GitHub, `gh pr view --json mergeStateStatus` is a third: `BEHIND` means behind, `DIRTY` means it already conflicts with the base, and `UNKNOWN` means GitHub is still working it out, so ask again until it says something else rather than reading it as current. On Azure Repos, rely on `jidoka pr sync` for the base state.

Whenever you add to the branch, whether to resolve a conflict or to fix CI, make a new commit and never rebase, amend or force-push. The review is pinned to a commit hash, and rewritten history would leave that pin, and the reviewers' comments, pointing at commits that no longer exist.

## When CI fails

When the outcome is `failed`, read the failure details it printed (fetch the full log only when the excerpt is not enough), fix the cause in a new commit on the same branch (never amend or force-push, as above), run the project's full verification on the changed code before pushing, formatting included and not just the failing check, then push, note it with a milestone update, and run `jidoka pr wait` again; repeat until CI is green. CI fixes are not Reworks.

- After `head-changed`, run it again.
- For `cancelled` or `timeout`, look at the checks it lists; if the cause is not something you can fix, treat it as a failure you cannot fix.
- A failure you cannot fix, such as an infrastructure problem, a missing permission, or a flaky external dependency, blocks the run with that reason.
- On `no-ci`, skip the watch and say so.
- On `not-open`, report that the pull request was merged or closed and stop; the run has already followed it, so do not update it again.

## Reporting the result

Never merge the pull request: merging is the human's review decision, and the run ends at a pull request that is ready for review. Report the pull request as ready for review only when the outcome is `passed` and the pull request is not behind its base. Keep the worktree for review follow-ups. Report its link, the worktree path (after the pull request merges, `jidoka run cleanup <id>` removes the worktree and local branch), the checks performed, and the final CI result. If a required tool, permission, or repository configuration is unavailable, report what remains, mark the run blocked with that as the reason, and stop.
