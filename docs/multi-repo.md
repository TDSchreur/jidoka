# Changes across repositories

A platform change can touch several repositories: a Bicep module, the infrastructure that uses it, an API. It can need several pull requests, and publication or deployment steps between them. Each Jidoka run keeps one repository, one worktree, one branch and one pull request. A **change group** links those ordinary runs to one shared spec and records what each run waits on. Groups are advisory: they never change a run's stages, gates, verification or cleanup, and nothing blocks on a prerequisite.

This guide walks through one change. The command reference is in [Runs and the dashboard](runs.md#change-groups), and the rules are in [ADR-0012](adr/0012-change-groups-link-runs-through-a-server-owned-groups-file.md) and [ADR-0013](adr/0013-group-dependencies-are-advisory-conditions-stored-in-the-group.md).

## The shared spec

A group needs a title and a spec. For a team, use a tracker issue or a durable web link to a document in a platform repository. The spec is the team's index of the related pull requests and milestones. Jidoka stores groups on your machine only, so the spec is how teammates follow along; a teammate who wants a local group creates one against the same spec and attaches their own runs.

Keeping it current is people's work, not Jidoka's:

- Whoever opens or replaces a pull request adds or updates its link in the spec.
- Whoever confirms or reopens a milestone updates its status in the spec and includes the evidence URL when there is one.
- Each pull request links the spec, so a reviewer can find the current status after the agent has finished.

For individual use the spec can be a local file. Jidoka stores its absolute path, so later commands work from any checkout. Because reviewers cannot open it, the pull request describes the prerequisites in words and leaves the path out.

## Example: a Bicep module and two consumers

The change publishes `module v2.4.0`, which the infrastructure and the API then use.

1. Start a run in each repository as usual, then group them:

   ```bash
   jidoka group create "Bicep module v2.4.0" --spec https://example.com/platform/changes/module-2-4
   jidoka group add <group-id> <module-run>
   jidoka group add <group-id> <infra-run>
   jidoka group add <group-id> <api-run>
   ```

   You can also start with `--run <id>` on `create`, and attach other runs as you find what the change touches.

2. Create the milestone once, and link everything that waits on it:

   ```bash
   jidoka group milestone add <group-id> "Bicep module v2.4.0 published"
   jidoka group depend <group-id> <infra-run> --milestone <milestone-id>
   jidoka group depend <group-id> <api-run> --milestone <milestone-id>
   jidoka group depend <group-id> <infra-run> --on-pr <module-run>
   ```

3. Each run finishes on its own. In Finish, `jidoka run next` prints the group, the spec and the run's prerequisites, and the agent describes them in the pull request: the infrastructure pull request says it needs the module's pull request merged and `v2.4.0` published, and links the spec for the current status. Add each pull request's link to the spec.

4. The module pull request merges. Only the pull request dependency clears. The publication milestone stays pending, because merging does not publish. `jidoka group inspect <group-id>` shows `1 of 3 attached PRs merged, 1 milestone pending`.

5. Once the version is published, confirm it. Both consumers clear at once, and the summary counts the milestone once:

   ```bash
   jidoka group confirm <group-id> <milestone-id> --evidence https://example.com/feed/module/2.4.0
   ```

   Then update the milestone's status in the spec. If you confirmed in error, `jidoka group reopen <group-id> <milestone-id>` makes both consumers wait again and clears the time and evidence. Jidoka does not record who confirmed, only when.

If a requirement changes, add a new milestone, link the affected runs to it, and remove their old dependencies with `jidoka group undepend`. An unreferenced milestone can then be deleted with `jidoka group milestone remove`. This keeps evidence tied to the requirement that was verified.

## What `group inspect` shows

For each member: its repository, its pull request with the stored state and when it was last checked, and the dependencies under the run that waits, each with its ID and whether it is clear or pending. Milestones are listed with their IDs even when no run waits on them. The distinct states of a pull request dependency are:

| Shown | Meaning | Clears the dependency |
| --- | --- | --- |
| `no pull request recorded` | The upstream run has no pull request yet. | No |
| `unchecked` | The upstream run has a pull request, but no state has been read for its current URL. | No |
| `open`, `a draft` | The pull request is under way. | No |
| `closed without merge` | The pull request was closed. | No |
| `unknown` | The host could not be asked; the run's detail says why. | No |
| `merged` | The stored state for the current URL is merged. | Yes |
| `run missing` | The upstream run's record is gone (a hand edit of `runs.json`). | No |

A merge counts only for the pull request the run records now. If the run's pull request is replaced, the dependency is pending and `unchecked` until the replacement has its own state, so a previous pull request's merge cannot satisfy it.

## The dashboard view

The dashboard lists each group in a Change groups section below On the line. A row shows the title, the summary that `group inspect` prints and the spec. Expanding the row lists the member runs with their pull request and its state in the words `group inspect` uses (shown in words, not only colour), when the state was last checked, the dependencies with `cleared` or `pending`, and the milestones with their confirmation time, evidence link and waiting runs. The runs stay in their repository views and are not counted again; a run's row and detail name its group. The view is read-only: use the `jidoka group` commands to change a group. See [Runs and the dashboard](runs.md#change-groups).

## How pull request states refresh

Group inspection and Finish only read what the dashboard server stored; they never ask the host and never touch a worktree. The stored state is refreshed two ways:

- **In the background.** While the dashboard runs, it reads each run's pull request state about once a minute until the pull request is merged. Keep `jidoka dashboard` running to see states change.
- **On demand.** `jidoka run sync <run-id>` (and `jidoka pr wait` when a pull request is no longer open) reads the host now, and the dashboard's Check PR button does the same. Use it right after you merge, when a dependency still reads `open`.

Without the host's tool signed in (`gh`, or `az` with its `azure-devops` extension) the state stays `unknown`. See [Pull requests and CI](pull-requests.md).

## Cleanup and detached members

- A cleaned-up run (`jidoka run cleanup <run-id>`) keeps its run record, so it keeps its place in the group and still counts its recorded pull request state.
- A run in a group cannot be deleted from the dashboard; the server tells you to detach it first.
- `jidoka group remove` refuses to detach a run that still waits on something or that another run waits on, and lists the dependencies. Remove them with `jidoka group undepend`, then detach. `jidoka group delete` removes only a group with no runs.
- A member whose run record is gone shows as `run missing`. Detach it with `jidoka group remove`.

## Optional: a platform documentation repository

An ordinary checkout of a platform repository can hold an architecture overview, repository links and owners, platform ADRs, environment conventions and a `changes/<name>.md` spec per change. Link it by its web address so teammates can open it. Use ordinary checkouts, one per repository, so each keeps its normal worktree and pull request workflow. Jidoka refuses to create a worktree for a repository whose git folder lies inside another `.git` folder, so submodules do not work for day-to-day runs; use them only for a tested release snapshot.
