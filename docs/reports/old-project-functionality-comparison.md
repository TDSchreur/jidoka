# Functionality comparison with the original TDS Factory

Date: 2026-11-05.

The new project preserves the core idea-to-PR workflow, isolated concurrent runs, agent skills and a shared progress dashboard. Its largest functional gap is the machinery around a run: durable context, enforced evidence, recovery, PR follow-up and measurements. Much of the old project's behavior is now an instruction to the agent rather than something the CLI records or enforces.

## Scope and evidence

This report compares implemented source and shipped workflow instructions at these revisions:

- Old project: [`TDSchreur/tds-factory`, `36128ce`](https://github.com/TDSchreur/tds-factory/tree/36128ce0052f6dc1625aaa848fe7ac278061852a), its fetched default branch, `develop`.
- New project: [`TDSchreur/tds-factory2`, `f319398`](https://github.com/TDSchreur/tds-factory2/tree/f3193984b7186165bcb3f6c7b987af0c97af725c), matching the checkout and fetched `origin/develop` at inspection.

This is a source comparison, not a live execution of both products. An implemented capability does not establish that every agent/client combination has been validated. PR #88's worktree adoption changes were reviewed separately and are not included in the new baseline above.

The old [CLI dispatcher](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/cli.js) exposes the run operations below. The new [CLI dispatcher](https://github.com/TDSchreur/tds-factory2/blob/f3193984b7186165bcb3f6c7b987af0c97af725c/src/main.rs) exposes installation, dashboard launch, run creation and run updates. The new [run model](https://github.com/TDSchreur/tds-factory2/blob/f3193984b7186165bcb3f6c7b987af0c97af725c/src/run.rs) stores title, project path, stage, status, activity, current ticket, blocking reason, rework count and update time. That deliberately small model explains several gaps.

## Missing or reduced capabilities

Priority is a recommendation for this project's simple, fast design, not an existing commitment. P1 means restore a small useful version first; P2 means add when a demonstrated need appears; P3 means retain as an optional future capability.

### Run correctness and continuity

| Capability in the old project | New project today | Practical consequence | Priority |
| --- | --- | --- | --- |
| Read-only readiness checks through `doctor`, including tracker, remote, verification commands, guides, installed skills and agent capabilities; JSON output | Setup and workflow instructions handle prerequisites; no diagnostic command | Users discover setup failures during a run instead of in one readiness report | P1 |
| Durable run context: branch, worktree, base commit, spec, ticket list and progress, PR, stage summaries and status reasons | A durable progress record exists, but lacks these structured links and execution context | Another session cannot reconstruct the work reliably from the run record alone | P1 |
| Named transitions that enforce stage order and prerequisites | The agent can report any valid stage/status combination directly | A dashboard update cannot establish that Define, Build or Check requirements were satisfied | P1 |
| Verification command execution, saved output and result tied to a clean Git head; stale evidence invalidation | Skills require project checks and final-commit verification, but results are not represented or enforced in the run model | The system cannot detect a claimed pass for older code | P1 |
| Stored review verdicts and acceptance tied to the reviewed code, checked before Finish | Reviews and user decisions happen through skills/chat | No durable, machine-checked distinction between reviewed, accepted-with-findings and unreviewed code | P1 |
| `run inspect`, proposed `run repair`, interruption recovery and `/factory continue <id>` | No inspect, repair or explicit continuation protocol; blocked is the only unfinished exception status | Recovery relies on chat history and manual inspection | P1 |
| Distinct waiting, interrupted, verification-unavailable and abandoned states | Running, blocked and done | Human decisions, unavailable tools, interrupted agents and cancelled work are difficult to distinguish | P2 |

Evidence: old [lifecycle implementation](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-lifecycle.js), [verification and review evidence](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/evidence.js), and [repair implementation](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-repair.js). These checks constrain supported CLI operations; the old system is not tamper-proof against an agent that edits its files directly.

### Review and delivery

| Capability in the old project | New project today | Practical consequence | Priority |
| --- | --- | --- | --- |
| First-class Follow-up stage observing PR conflicts, failing checks and unresolved threads, with scoped rework on the existing PR | Finish watches CI and fixes failures before reporting done; later follow-up has no dedicated protocol | Review comments and post-completion failures require manual coordination | P1 |
| GitHub and Azure DevOps PR adapters for status, checks and follow-up | No equivalent host adapter or Azure status/follow-up integration | Azure users lack the old integrated PR support; generic agent capabilities may still allow manual work | P2 |
| Dedicated Infrastructure review axis and project-configured source/UI/UX reviews with frozen skill snapshots | Standards/spec review and security review exist; no configured project review framework | Additional stack-specific reviews depend on separate project instructions | P2 |
| Installed stage agents and explicitly restricted read-only reviewers for Claude, Codex and Copilot | Skills are installed, without those named stage-agent configurations or review sandbox contracts | Reviews can still use subagents through the review skill, but lack the old installed isolation guarantees | P2 |
| Recorded distinction between blocking findings and LOW/INFO/style items left open | Workflows route actionable findings through at most three reworks, then ask the user | The old severity/acceptance policy is not reproduced as structured state | P2 |
| `unslop` pass over the PR title/body | `pr` skill without the extra cleanup skill | One optional prose-quality pass is absent | P3 |

Evidence: old [Follow-up implementation](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-follow-up.js), [PR provider selection](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/pr-provider.js), [project review guide](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/docs/project-reviews.md), and [agent capability matrix](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/docs/capabilities.md). New behavior is defined in the [three workflow skills](../../.agents/skills/).

### Workspace and scope management

| Capability in the old project | New project today | Practical consequence | Priority |
| --- | --- | --- | --- |
| CLI-managed worktree/branch identity stored with the run | Agent creates and retains its dedicated worktree; run record lacks its identity | Files remain isolated, but lifecycle operations cannot locate them from the record | P1 |
| `run sync` fetch/rebase, conflict checks before Finish, evidence invalidation after changes | No equivalent managed command | Keeping long-running branches current requires agent/manual Git work | P2 |
| `run clean` after a merged PR, preserving dirty work and extra commits; coordinated deletion of Factory-owned worktrees | Manual `git worktree remove`; dashboard deletes done records only and leaves files/branches | Old run workspaces accumulate unless users clean them up themselves | P2 |
| Diff-size measurement, waiting for a split/continue decision and handing remaining tickets out of the run | No run-size gate or split protocol | Large runs can grow without an explicit checkpoint for reviewability | P2 |
| Canonical project identity based on the main checkout, including launches from linked worktrees | Project identity is the reported current directory path | Runs started in separate worktrees of one repository can appear as separate Projects | P1 |

Evidence: old [worktree management](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/worktrees.js), [cleanup](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-clean.js), [size measurement](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-size.js), and the new [project grouping implementation](../../dashboard/app.js).

### Dashboard and measurements

| Capability in the old project | New project today | Practical consequence | Priority |
| --- | --- | --- | --- |
| Live PR checks/review/conflict status and Check PR/Fix actions | Activity can contain a PR URL; no structured PR status or actions | Users must open the host or ask an agent to discover blockers | P1 |
| Ticket progress, spec/ticket/PR links, rendered local documents and stage summaries | One current-ticket URL and activity; basic run-detail drawer | The dashboard tracks current activity but cannot present the full delivery context | P1 |
| Event timeline, last-seen observations and quiet-agent warnings | Latest update timestamp only | No history of stage visits, verification, waiting or rework decisions | P2 |
| Stage durations, rework causes, first-pass clean rate and totals via `run stats` | Latest rework count and timestamp | No outcome or throughput comparison across runs | P2 |
| Per-stage/model/ticket token usage from Claude, Codex and Copilot session logs | Per-stage, per-model usage is read from the three agents' logs; no per-ticket breakdown | Token cost per stage and model is measurable; per-ticket cost is not | P3 |
| Sync, Accept Check, extra-loop grants, Split/Continue and Clean up actions | Delete done records; other decisions remain in chat | The dashboard is mainly an observer rather than a run-control surface | P2 |
| Older-run filtering and waiting duration | Needs-you filtering exists, but no age filter or waiting-age history | Long-lived dashboards offer less history management | P3 |
| Persistent project registry, empty/missing-project management and automatic free-port fallback | One shared default-port dashboard, grouping projects from stored runs; explicit alternate port | Empty projects are not registered, worktree paths fragment identity, and a busy foreign port requires intervention | P3 |

Evidence: old [dashboard guide](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/docs/dashboard.md), [metrics](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-metrics.js), [usage accounting](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-usage.js), and the new [dashboard](../../dashboard/app.js). The old system counts tokens, not money; financial cost reporting is not a lost feature.

### Configuration, installation and maintenance

| Capability in the old project | New project today | Practical consequence | Priority |
| --- | --- | --- | --- |
| Project options frozen per run: stage models, loop cap, ticket labels, usage and extra reviews | Three fixed workflow modes, project instructions, and a model-routing policy for Claude Code, Codex and Copilot CLI whose selection is recorded on the run | Stage models are covered; loop cap, ticket labels and extra reviews have no configuration contract or snapshot | P2 |
| Project-local installation, dry run, saved destination selection, multiple explicit Claude directories | Global agent selection; environment variables select Claude/Codex roots | Shared repositories and multiple profiles need more manual setup | P2 |
| Ownership/change detection, preservation of edited or colliding skills, force override, cleanup/backups for deselected destinations and install-path trust checks | Bundled directories are replaced on install; unrelated skill names remain | Same-name local changes can be overwritten; switching destination selection has fewer safeguards | P1 |
| Dedicated checksum-verified `update` command | Re-running the checksum-verified one-line installer updates CLI and skills | Updating remains possible, but has no dedicated CLI entry point or no-force mode | P3 |
| `rules` converts scoped Copilot instruction files into Claude rules; `run guides` resolves applicable guides | Scoped project guides are documented; no conversion/resolution commands | Agents rely more on reading and following project instructions themselves | P2 |
| `run sweep` finds stale terminology, changed numbers, list members and claims across docs/code help | General documentation responsibilities and review | No automated cross-document drift discovery | P2 |
| Live-agent evaluation harness with offline host fixtures and workflow-adherence scenarios | Unit tests and CI; no equivalent agent-flow evaluation harness | Passing code tests does not establish that agents follow the complete workflow | P1 |
| Upstream skill synchronization command and provenance workflow | Committed skills with upstream/license information, without the old sync command | Updating vendored skills needs manual maintenance | P3 |

Evidence: old [installer guide](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/docs/install.md), [project configuration](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/project-config.js), [term sweep](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/src/run-sweep.js), [evaluation harness](https://github.com/TDSchreur/tds-factory/blob/36128ce0052f6dc1625aaa848fe7ac278061852a/docs/evaluation.md), and the new [installer](../../src/install.rs) and [bundled skill list](../../build.rs). The old live-agent baseline covers only a subset of scenarios on Claude; it is not proof of full Codex/Copilot compatibility.

## Differences that should not automatically be restored

- The new project has three explicit modes: quality, balanced and fast. The old Fast track still required Standards and Security reviews and a spec/ticket. The new fast mode intentionally skips Check and can work directly from a clear request. These are different policies, not an absent Fast mode.
- The new workflows use sequential implementation. Per-stage models are now available through model routing on all three agents. The old stage dispatch and restricted review agents are useful capabilities, but recreating the entire coordinator architecture would increase complexity.
- The old Build exit checklist required story traces, a security-boundary account, hostile-input checks, a term sweep and extensive repeated test runs. The new project retains testing, reviews and required project checks, with less mandatory ceremony. Restore targeted checks when they prevent observed failures; avoid importing that entire checklist by default.
- `diagnosing-bugs` exists in the new source tree but is not bundled by `build.rs` or explicitly integrated into the fast workflow. The old Fast track invoked it for bugs. `reliable-tests`, `add-project-review` and `unslop` are also absent from the new shipped bundle. Domain modeling, TDD, code review, security review and PR writing already have equivalents; renaming `security-scan` to `security-review` is not a missing scan.
- Native Rust installation removes the old Node/npm runtime requirement. Linux/Windows installation, Claude/Codex/Copilot targets, checksums, themes, responsive project rows, a detail drawer, needs-you filtering, browser notifications and done-record deletion already exist.
- Both projects isolate runs in worktrees and leave merge decisions to the user. Multiple concurrent factories are supported. The new Finish stage also explicitly watches CI and fixes failures before marking the run done.

## Remaining differences between Claude Code, Codex and Copilot

Model routing works on all three agents. These differences remain:

- **Installed stage agents and read-only reviewers.** The old project installed named stage agents and restricted read-only reviewers for each agent. The new project installs skills only, for all three agents; reviews and routed stages use subagents started by the skills, without installed isolation or sandbox contracts.
- **Model selection.** Claude Code accepts only the aliases `opus`, `sonnet`, `haiku` and `fable` through the Agent tool. Codex uses its subagent model and effort settings. Copilot CLI uses exact model IDs through the `task` tool, and an unavailable model silently runs on the session's model.
- **Readiness checks.** `factory doctor` checks the Claude Code version and its routed models and efforts. For Codex and Copilot CLI it cannot tell which versions support per-subagent models, and for Codex it keeps no model list, so those checks are skipped.
- **Pause detection.** Usage limits and context compactions are detected from Claude Code logs only. Codex and Copilot runs are never blocked as paused; only idle gaps show.
- **Validation.** The old live-agent baseline covers only a subset of scenarios on Claude. The Copilot agent-flow evaluations are marked unvalidated until run.

## Suggested implementation order

1. **Record enough context to resume.** Add canonical repository path, worktree, branch, base SHA, mode, spec/tickets and PR link. Expose that context in run detail and provide a small inspect/continue protocol.
2. **Make verification trustworthy.** Persist the command, exit result and verified SHA. Reject completion when the recorded verification belongs to different code. Add review/acceptance evidence incrementally rather than reproducing every old gate.
3. **Support the review loop after publication.** Read current PR blockers, retain the existing PR and hand scoped follow-up work back to an agent. Begin with the host used by the project; add Azure parity when needed.
4. **Improve readiness and installation safety.** A small doctor command and preserving locally edited skill directories prevent common failures without expanding the workflow much.
5. **Add a few agent-flow evaluations.** Start with failed verification, stale evidence, interrupted/resumed work and an already-created PR. Test Claude, Codex and Copilot separately and label unvalidated cases.
6. **Measure before optimizing.** Start with stage history/durations and rework causes; add token accounting if it will guide model or mode choices. Leave the larger dashboard-control and project-review frameworks until actual usage justifies them.

The first two steps provide the foundation for most later capabilities. A compact run model with explicit identity and evidence would recover much of the old project's practical reliability without restoring its full process overhead.
