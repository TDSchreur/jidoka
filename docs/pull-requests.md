# Pull requests and CI

The `jidoka pr` commands open a run's pull request, keep it current with its base and watch its CI, on GitHub and Azure Repos.

## Opening a pull request

`jidoka pr open <run-id> --title "<title>" --body-file <path>` pushes the run's branch and opens its pull request, or finds the one that is open, and records its URL on the run. The agent still writes the title (one line, up to 256 characters) and the description (a file of up to 256 KiB). For GitHub it needs the GitHub CLI (`gh`), signed in with `gh auth login`. For Azure DevOps Services it needs the Azure CLI (`az`) and its `azure-devops` extension, authenticated with `az login` or `az devops login`. Git also needs credentials for the remote.

```text
jidoka pr open <run-id> --title <title> --body-file <path> [--base <branch>] [--json]
```

- **Base.** `--base` names the branch the pull request goes against; without it the command uses the branch `refs/remotes/origin/HEAD` points to.
- **Checks before anything is pushed.** The run has a worktree recorded and it exists; `gh` is installed; the worktree is on a branch (the one the run records, when it records one) whose name is safe to push; its tree is clean; the branch is not the base and has commits beyond `origin/<base>`; and `gh` is signed in and can see the repository, which is asked by looking for an open pull request before the push. A branch name that git could read as an option or a forced refspec, such as `+main`, is refused.
- **Push.** `git push -u origin refs/heads/<branch>:refs/heads/<branch>`, never forced. If origin rejects it because the branch has commits yours does not, the message says to merge them or the base into the branch with a normal merge and try again.
- **Retry.** It looks for an open pull request of the branch first, ignoring pull requests from forks. If there is one it is reused, so running the command again never makes a second pull request; new commits are still pushed to it. If another run of the command opens it between the look and the creation, the one that won is used. If the pull request opened but the server did not take the record, the message names the URL and a retry records it. For a run that is blocked or done the output says that the run was not updated and how to record the pull request.
- **Output.** `Pull request opened` or `Pull request already open`, the URL, the branch, the base and the pushed commit; `--json` prints the same fields (`pull_request`, `created`, `branch`, `base`, `commit`, `recorded`). It sets the run's pull request and the activity "Opened pull request" for a running run, keeping its stage, and leaves a run that is blocked or done alone. It never waits for CI: `jidoka pr wait` is the next step. Problems exit with status 1.

## Keeping a pull request current

`jidoka pr sync <run-id>` brings the run's branch up to date with its base, the way the Finish procedure asks for. Conflict resolution and verification stay with the agent.

```text
jidoka pr sync <run-id> [--base <branch>] [--json]
```

| Outcome | Exit status | Meaning |
| --- | ---: | --- |
| `current` | 0 | The branch already contains `origin/<base>`. Commits of yours that are not pushed are reported and left alone. |
| `pushed` | 2 | The base was merged with a normal merge commit, it touched none of the change's own files, the branch had nothing that was not pushed yet, and the merge was pushed. Watch CI again with `jidoka pr wait`. |
| `needs-verification` | 3 | The base merged cleanly, but it touched files of the change (listed), or the branch has commits that are not pushed yet, or it was never pushed. The merge is not pushed: rerun the project's verification on the merged code, then push. |
| `conflict` | 4 | The merge conflicts and is left in progress, with the conflicting files listed. Resolve them, rerun the verification, commit and push. |

Exit status 1 is a problem with what to do.

- **Checks.** The run has a worktree recorded and it exists; it is on the recorded branch, whose name is safe to push; the tree is clean; no merge is in progress; and the branch is not the base. The base is `--base`, otherwise the branch `refs/remotes/origin/HEAD` points to, and it is fetched first.
- **Behind.** `git merge-base --is-ancestor origin/<base> HEAD`: exit 0 is current, 1 is behind, anything else is an error and never read as behind.
- **Overlap.** The change's own files (`git diff --no-renames --name-only <merge base> HEAD`, taken before the merge) against the base's files since the merge base. Without rename detection a renamed file counts under both names.
- **Merge.** `git merge --no-ff` with the message "Merge origin/<base> into <branch>": never a rebase, amend or fast-forward. It never merges the pull request.
- **Push.** `refs/heads/<branch>:refs/heads/<branch>`, never forced. If the push fails the merge commit stays, and running the command again pushes it, but only when HEAD is that merge exactly as git makes it on its own, on top of what the remote has, with no overlap; a merge that needs verification or a conflict resolved by hand is never pushed by the command. If someone else pushed to the branch, the command stops before merging and says so.
- **Run record.** A running run gets a milestone activity, such as "Merged develop into the branch", on its current stage and status; it is neither a Rework nor a CI fix.

Azure Repos pull request creation recognizes `dev.azure.com` HTTPS URLs, `ssh.dev.azure.com` v3 SSH URLs, and legacy `visualstudio.com` HTTPS URLs. The Azure DevOps CLI does not support Azure DevOps Server. Azure descriptions must be UTF-8 and at most 4,000 characters; Jidoka reports the length and how much to cut. Jidoka hands the body to the CLI as `@<file>`, because Windows cannot pass a multi-line argument to `az.cmd`, and on Windows refuses a title or body file path with a double quote, percent sign or control character before pushing. Jidoka passes the organization, project, and repository explicitly to the CLI and checks for an existing pull request before pushing.

For a GitHub Enterprise host that cannot be identified from its remote, set `git config jidoka.prProvider github`. For a custom Azure Services remote, explicitly select the target with `git config jidoka.prProvider azure` and set `jidoka.azureOrganization`, `jidoka.azureProject`, and `jidoka.azureRepository` with `git config`. For an unrecognized custom remote, also set `jidoka.azureOrigin` to its exact `origin` URL, so stale configuration is rejected if the remote changes. For recognized Azure remotes, the configured destination must match `origin`. An unknown host without an override is refused.

The workflows' shared Pull request and CI procedure names the tool for each host. On any other host, the agent opens the pull request and watches CI with that host's own tooling and records the URL on the run with `--pr`.

## Watching CI

`jidoka pr wait <run-id>` watches the checks of the run's pull request and prints one CI outcome, so an agent does not poll with repeated model requests. It reads the pull request URL recorded on the run (`jidoka run update <id> running --pr <url>`) and uses the GitHub CLI (`gh`, signed in with `gh auth login`) for GitHub, or the Azure CLI with the `azure-devops` extension (signed in with `az login` or `az devops login`) for Azure DevOps Services.

| Outcome | Exit status | Meaning |
| --- | ---: | --- |
| `passed` | 0 | Every check of the current head commit passed or was skipped. |
| `failed` | 2 | A check failed. Each failed check is listed with its link. The first failed check of each Actions run, for at most three runs, also carries the last 40 lines (at most 4 KiB) of that run's failed log; a log that cannot be fetched leaves the name and link. A failure wins over a cancel. |
| `cancelled` | 3 | Checks were cancelled and none failed. |
| `timeout` | 4 | Checks were still running at `--timeout` (default 30 minutes, from 1 second to 24 hours, written as `90s`, `30m` or `2h`). |
| `head-changed` | 5 | The pull request got a new head commit while waiting. No check results are reported; run the command again. |
| `no-ci` | 6 | No check appeared within 90 seconds, or within `--timeout` when that is shorter. |
| `not-open` | 7 | The pull request is merged or closed. |

Exit status 1 is a problem with the tools or the run (no `gh` or `az`, not signed in, no network, no pull request recorded), with what to do. Checks are asked for every 15 seconds, and the progress goes to standard error. Right after a push, GitHub can still report the previous head commit for a few seconds. So when the run's worktree is on the commit its branch has on origin and the pull request shows another one, the command first gives GitHub up to 30 seconds to show the pushed commit, asking every 5 seconds, and then watches whatever head the pull request reports. That time counts toward `--timeout`. By default it waits until every check has ended; `--fail-fast` returns `failed` at the first failing check and lists the others as still running. `--json` prints the same data.

Next to the outcome it says whether the branch is up to date with its base, behind it, in conflict with it, or not yet known. GitHub is asked again up to five times. Azure reports a conflict when its merge calculation finds one and otherwise reports unknown, because a successful merge does not prove the branch is up to date; check it with `jidoka pr sync`. The command only reads: it never merges, pushes or updates the branch, and a `passed` outcome does not mark Finish done. While it waits, the run's activity is "Waiting for CI", and it is set to the outcome when the wait ends; a run that is blocked or done is left alone. Names, links and log lines come from the pull request's checks and are printed with control characters replaced. Ctrl-C ends the process; the activity text then stays until the next update of the run. The Finish procedure of the workflows uses this command. When the installed `jidoka` does not know a command the procedures use, the run stops and reports that Jidoka needs upgrading with `jidoka install`.

For Azure DevOps Services, record the canonical `https://dev.azure.com/<organization>/<project>/_git/<repository>/pullrequest/<id>` URL; a legacy `visualstudio.com` URL is refused with a request for the canonical one. Jidoka checks the source branch against the latest PR iteration and merge calculation before it accepts results, and checks it again after collecting them. It watches enabled, required policies and statuses of the current iteration:

- Approved policies and successful statuses pass.
- Rejected or broken policies and failed statuses fail. A rejected build policy is reported as cancelled only when its build run says it was cancelled.
- Queued, running, expired, or unknown states remain pending.
- A build policy counts only when its evaluation started at or after the current iteration; a missing or older start stays pending, even when Azure reports an approval. Other policies, such as minimum reviewers or linked work items, are re-evaluated by Azure, so their reported state counts as is.
- PR-wide statuses without an iteration cannot prove a result for the current head and remain pending, whatever state Azure reports for them. A newer status in the same context replaces an older one. So a pull request that has a PR-wide status, from any integration, never reaches `passed` or `failed`: the wait ends in `timeout`, or in `failed` with `--fail-fast` once another check fails.
- Optional policies are shown but do not block. Disabled and not-applicable policies do not count as CI.

Failure links point to the build when there is one, otherwise to the PR. Failed log excerpts are only available for GitHub Actions, and PRs from Azure forks are not supported.
