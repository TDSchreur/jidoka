---
name: jidoka-create-release
description: Create a new release of this project end to end and unattended: pick the next version, bump it through a pull request into develop, open and merge the develop-to-master release pull request, watch the release workflow and verify the published assets. Use when the user asks to release, cut a release, publish a new version, ship a version or bump and release, even if they do not name the steps.
disable-model-invocation: true
argument-hint: "[patch|minor|<version>]"
---

# Create a release

Release the current `develop` following [docs/releasing.md](../../../docs/releasing.md). Read it first; this skill is the unattended way through the same steps.

Invoking this skill is the user's authorization to merge the version-bump pull request and the release pull request, and so to publish. That overrides the usual rule that runs never merge. Publishing is a one-way door: the workflow tags and releases on merge, and tags cannot be moved. So check before the release pull request is merged, and stop instead of guessing when something is off.

Work from the main checkout (the first entry of `git worktree list --porcelain`) except where the bump run says to use its worktree. Never push tags, run `gh release create` or edit the release by hand: the workflow must do it, because it builds and tests both binaries first.

## 1. Check the starting point

```sh
git fetch origin --tags
gh run list --branch develop --limit 3
gh pr list --base master --head develop --state open
```

Stop and report when any of these hold:

- `develop` has nothing that `master` lacks: `git log --format=%s origin/master..origin/develop` is empty or has only subjects like `Release <version>: bump version`.
- The CI run for the head commit of `origin/develop` failed. If it is still running, wait with `gh run watch <run-id> --exit-status`; do not release over a failure.
- A release pull request from `develop` to `master` is already open. Report it and ask whether to continue it instead of opening a second one.

## 2. Choose the version

Take the version from the argument: `<version>` is used as is, `patch` or `minor` bumps the current `Cargo.toml` version on `develop`. With no argument decide from what is shipping, because the project is on 0.x and a breaking change shows up as a minor bump:

- Minor: a `changelog.d/` fragment added since the last tag that removes or renames a command, option or skill, or says what a user must now do (usually under `## Changed`), or a new user-visible feature under `## Added`.
- Patch: everything else.

Find the fragments with `git diff --name-only <last-tag>..origin/develop -- changelog.d` and read them. The version must be greater than the last tag, and `git ls-remote --tags origin v<version>` must show nothing; if the tag exists, stop. State the version and the reason in one sentence and carry on; do not ask for approval of a routine choice.

## 3. Bump the version through a run

Repository changes go through jidoka, so start a Fast run on `develop`: `jidoka run begin "Release <version>: bump version" --agent <the agent you are running in: claude, codex or copilot> --mode jidoka-fast --base develop`, and work in its worktree.

1. Set `version` in `Cargo.toml` and run `cargo check` so `Cargo.lock` follows. The diff must touch only the version in those two files; if it touches more, find out why before going on.
2. Run `scripts/verify.sh --quick`. No changelog fragment and no ADR is needed for a version bump; say so in the pull request.
3. Commit as `Release <version>: bump version`, then take the run through its Finish stage as the run's instructions say (they cover `jidoka pr open`) until the pull request is open and `jidoka pr wait <run-id>` reports `passed`.
4. Merge it with a squash once `ci-ok` passes (`gh pr merge <number> --squash`). When `develop` moved and the branch is behind, use `jidoka pr sync <run-id>`, watch CI again with `jidoka pr wait <run-id>`, then merge.
5. Run `jidoka run cleanup <run-id>` from the main checkout and `git pull --ff-only` on `develop`.

## 4. Open the release pull request

Write the description with the `jidoka-pr` skill if it is available, in this shape:

```markdown
## Release <version>

Merges `develop` into `master` (merge commit) to publish v<version>.

## Changes

<one bullet per commit from: git log --first-parent --format='- %s' origin/master..origin/develop>

## Notes for users

<only when a fragment tells users to act, for example restart the dashboard; otherwise leave the section out>

## Merge danger

One-way door: the release workflow tags and publishes on merge, and tags cannot be moved. The version bump (#<number>) is already on `develop`, and the `v<version>` tag does not exist yet.
```

Save the description to a file in the scratchpad or a temporary directory and open it with `gh pr create --base master --head develop --title "Release <version>" --body-file <file>`. Do not delete the `develop` branch at any point.

## 5. Merge it

Wait for `ci-ok` with `gh pr checks <number> --watch`; if it says no checks are reported yet, wait a few seconds and run it again. If checks fail, read the failing logs and report them; `develop` must be fixed through its own run, then start again at step 1. Do not fix it inside the release.

Re-check `git ls-remote --tags origin v<version>` is still empty, then merge with a merge commit so the branches keep their shared history: `gh pr merge <number> --merge`. Never squash or rebase this pull request, and never pass `--delete-branch`.

## 6. Watch the release workflow

Take the merge commit with `gh pr view <number> --json mergeCommit --jq .mergeCommit.oid`. The run for it can take a few seconds to appear, and an older release run would be the wrong one, so poll until `gh run list --workflow release.yml --branch master --commit <sha> --limit 1` shows a run, then:

```sh
gh run watch <run-id> --exit-status
```

On failure read the failed logs (`gh run view <run-id> --log-failed`) and check whether the tag exists (`git ls-remote --tags origin v<version>`).

- Tag absent and the failure looks transient (runner or network problem): `gh run rerun <run-id> --failed`, once. Report the second failure instead of retrying again.
- Tag present: stop and report. A half-published release needs the user's decision.

## 7. Verify the release

`gh release view v<version> --json tagName,targetCommitish,assets --jq '.assets[].name'` must list all six assets: `jidoka-linux-x86_64`, `jidoka-linux-x86_64.sha256`, `jidoka-windows-x86_64.exe`, `jidoka-windows-x86_64.exe.sha256`, `install.sh` and `install.ps1`. Report any missing one as a failed release rather than a finished one.

## Report

End with the version and why it was chosen, links to the bump pull request, the release pull request, the workflow run and the release, and the asset check. If a step stopped the release, say which step, why, and what is already done (for example "bump merged, release pull request still open"), so the user can continue from there. Suggest `JIDOKA_AGENT=all scripts/update-local.sh` to update the local install; do not run it unasked.
