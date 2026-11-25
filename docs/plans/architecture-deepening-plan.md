# Architecture deepening plan

This plan comes from an architecture review of the Rust code in `src/`. Each step turns a shallow or leaky module into a deeper one, using the vocabulary of [`jidoka-codebase-design`](../../skills/jidoka-codebase-design/SKILL.md) (module, interface, depth, seam, adapter, locality) and the domain language in [GLOSSARY.md](../../GLOSSARY.md).

Work through it with [`jidoka-work-through-plan`](../../skills/jidoka-work-through-plan/SKILL.md) in a fresh session, giving it the path of this file. Each step is one pull request.

## Rules for every step

- Start with the `jidoka` skill and let it pick the workflow. A step is a refactor that keeps behavior: prefer `jidoka-balanced`, and use `jidoka-fast` only when the step turns out to be mechanical.
- Pull requests target `develop`. Do not touch `master`.
- Keep behavior the same. A step that must change user-visible behavior, a file format or the dashboard API stops and reports **BLOCKED**, because that needs an ADR and a decision from the user.
- Keep the existing tests passing, and move them to the new interface when the code they test moves. Add tests only for behavior a step newly exposes. Do not use TDD.
- Follow [CODING_STANDARDS.md](../../CODING_STANDARDS.md) and [AGENTS.md](../../AGENTS.md). Keep GitHub and Azure DevOps, Claude, Codex and Copilot, and Linux and Windows at the same level of support.
- Apply the deletion test before adding a module: if deleting it would only move complexity, do not add it. Do not add a seam that has only one adapter.
- Use the terms in [GLOSSARY.md](../../GLOSSARY.md). When a step names a concept the glossary lacks, add the term in the same pull request.
- Run `scripts/verify.sh --quick` while iterating and `scripts/verify.sh` before opening the pull request.
- State in the pull request which ADR it adds or follows, or that none was needed. Add a `changelog.d` fragment only when users can see a change.
- After the merge, finish from the main checkout: `jidoka run cleanup <run-id>`, then `JIDOKA_AGENT=all scripts/update-local.sh`.

Steps are ordered by recommendation. Steps 1 to 6 are independent unless a step says otherwise. Steps 7 and 8 come last because they move files that the earlier steps change.

## 1. One Git adapter for every `git` call

Prerequisite: none.

- [x] `knowledge.rs` and `doctor.rs` start `git` through the `Git` interface instead of their own `Command::new("git")`
- [x] Hermetic environment and error trimming come from one place (`git.rs`); `first_line` exists once
- [x] Tests share one git-repo fixture instead of the copies of `run_git` and `git()` in the `*_tests.rs` files
- [x] `scripts/verify.sh` passes
- [x] PR merged into `develop`

Run / PR / verification: run 1795632957778183281, [#607](https://github.com/TDSchreur/jidoka/pull/607) merged as ff6d8d0; `scripts/verify.sh` and CI passed. Deviation: doctor and knowledge now get the hermetic git environment, so "cannot run git" became "git could not be started".

```text
Deepen the Git adapter so every git call in src/ goes through the Git interface in src/git.rs.

Problem: git.rs defines trait Git with RealGit and a hermetic command() (it sets GIT_TERMINAL_PROMPT and LC_ALL and removes GIT_DIR). But knowledge.rs (around line 158) and doctor.rs (around lines 218 and 232) start git themselves without that environment and trim errors differently. first_line exists in git.rs and in pr_host/github.rs. About 7 test files each rebuild the same helper (run_git in pr_sync_tests.rs, run_check_tests.rs, run_cleanup_tests.rs and pr_open_tests.rs; git() in run_begin_tests.rs, knowledge_tests.rs and repository.rs), and client.rs and run_gate.rs repeat a 10-line hermetic test setup.

Goal: route knowledge and doctor through Git, give them a fake Git in their tests, keep one first_line, and add one shared test fixture for creating a git repository, used by the test files above. Do not add a new seam: Git already has a real adapter and fakes.

Files: src/git.rs, src/knowledge.rs, src/knowledge_tests.rs, src/doctor.rs, src/doctor/tests.rs, src/repository.rs, src/pr_host/github.rs, src/pr_open_tests.rs, src/pr_sync_tests.rs, src/run_check_tests.rs, src/run_cleanup_tests.rs, src/run_begin_tests.rs, src/client.rs, src/run_gate.rs.

Done when: no Command::new("git") remains outside src/git.rs and its tests, doctor and knowledge run through Git, the duplicated test helpers are gone, and behavior is unchanged.
```

## 2. A command interface for `main.rs`

Prerequisite: Step 1 merged.

- [x] Each command has one shape (parse options, run, render); `run_cli` only dispatches
- [x] Run id validation is one function used by every command, including `run gate` and `run advance`
- [x] `Hosts` and `RealTools` are built in one place for the CLI and the server
- [x] Logic that leaked into `main.rs` (`apply_pr_state`, the NotOpen follow-up in `pr_wait`, `agent_sessions_from`, dashboard port choice) lives in its own module with tests at that module's interface
- [x] `scripts/verify.sh` passes
- [x] PR merged into `develop`

Run / PR / verification: run 1795633757742974094, [#608](https://github.com/TDSchreur/jidoka/pull/608) merged as 53ce241; `scripts/verify.sh` and CI passed, old and new binaries matched on 95 command lines. Deviation: `run advance` prints "Advanced run N" after reading the new stage, so a rare knowledge-repo warning on stderr can now appear before it; also touched inspect, stats, requests, requests_pool, run_next, doctor, pr_host and CODING_STANDARDS.md.

```text
Make main.rs a thin dispatcher by giving each CLI command one shape.

Problem: run_cli (src/main.rs, lines 98-320) is one large match. Handlers such as pr_open_command, run_check_command, run_sync_command, run_cleanup_command, pr_sync_command, pr_wait_command and run_begin_command each repeat the same steps: parse options with a HELP-suffixed error, check the run id with client::check_run_id, fetch the run, build pr_host::Hosts::new(RealTools) and RealGit, and render JSON or text. Hosts::new(RealTools) is built 4 times in main.rs and 3 times in server.rs. The run gate and run advance arms check the run id inline instead of calling client::check_run_id. Business logic has leaked in: apply_pr_state, the NotOpen follow-up and activity reporting in pr_wait_command, agent_sessions_from (which overlaps run_begin::agent_homes), and the dashboard port choice. Each module's parse_options returns a different ad hoc type.

Goal: define a small per-command interface (parse, run, render) and make run_cli dispatch on it; move the leaked logic into the modules that own it, with tests there instead of in main.rs. Keep every command line, flag, output and exit code the same.

Files: src/main.rs, src/client.rs, src/pr_open.rs, src/pr_sync.rs, src/pr_wait.rs, src/pr_state.rs, src/run_begin.rs, src/run_check.rs, src/run_cleanup.rs, src/run_gate.rs, src/server.rs.

Done when: main.rs holds dispatch and nothing else, the run id check and Hosts construction each exist once, existing CLI tests pass, and the tests of leaked logic moved with it.
```

## 3. Split `usage.rs` and give timestamps one home

Prerequisite: none.

- [ ] Timestamp parsing and formatting live in one module used by `usage`, `agent_logs` and every other caller
- [ ] `civil_day` in `agent_logs.rs` and the date math in `usage.rs` are one implementation
- [ ] Shell-option parsing (`skip_git_options` in `request_logs.rs`) lives in `shell_commands.rs`
- [ ] The token-usage and request-analysis modules are grouped in a directory with the pipeline readable from its top
- [ ] `scripts/verify.sh` passes
- [ ] PR merged into `develop`

Run / PR / verification: —

```text
Split the dumping-ground usage.rs and give timestamps one home.

Problem: usage.rs (about 1064 lines, plus usage_tests.rs at about 1697) holds the Tokens, UsageRow, Usage, Pause and IdleGap types, date math (days_from_civil, parse_timestamp, format_timestamp, iso_timestamp), pause derivation, collection (collect, collect_and_store, collect_active, collect_missed), an env gate (UsageSettings) and run-window helpers. Six call sites outside the module use its timestamp helpers. agent_logs.rs has its own civil_day. request_logs.rs has skip_git_options, which repeats shell-parsing knowledge that shell_commands.rs owns. The pipeline agent_logs -> session_attribution -> usage / requests / requests_pool / request_logs -> stats / cost / attention is 10 flat top-level modules.

Goal: move timestamp helpers into one module and make all callers use it; move shell-option parsing into shell_commands; group the usage pipeline under a directory. Do not split usage.rs just because it is long: split it where the concerns differ (types, time, pause derivation, collection). Keep Token usage numbers byte-for-byte the same, since the glossary says unknown usage is never shown as zero.

Files: src/usage.rs, src/usage_tests.rs, src/agent_logs.rs, src/request_logs.rs, src/shell_commands.rs, src/requests.rs, src/requests_pool.rs, src/stats.rs, src/session_attribution.rs, and the callers of the usage timestamp helpers.

Done when: there is one timestamp implementation, one place for shell-option parsing, the tests still pass and moved with their code, and docs/usage-tracking.md still matches the code.
```

## 4. One interface for reading agent logs

Prerequisite: Step 3 merged.

- [ ] `Agent` is defined in [GLOSSARY.md](../../GLOSSARY.md)
- [ ] Reading logs for Claude, Codex and Copilot goes through one interface; `match Agent::...` dispatch in `agent_logs.rs` is gone
- [ ] Per-agent facts (home, log roots, attention rules) sit behind the same interface where it removes repeated matches in `attention.rs` and `routing/policy.rs`
- [ ] Adding an agent means one new adapter, not edits in many modules
- [ ] `scripts/verify.sh` passes
- [ ] PR merged into `develop`

Run / PR / verification: —

```text
Give the three agent log readers one interface.

Problem: agent_logs.rs re-exports read_claude, read_codex and read_copilot from agent_logs/claude.rs, codex.rs and copilot.rs. There is no trait: LogRoots has one field per agent, and match Agent::Claude/Codex/Copilot appears in agent_logs.rs (lines 74-96), attention.rs (about 12 matches) and routing/policy.rs (about 15). A fourth agent would need edits in many places. The project requires equal support for Claude, Codex and Copilot.

Goal: define the glossary term Agent, then give the log readers one interface so dispatch is not repeated. Move per-agent facts behind it only where that removes real repeated matches; do not force routing policy into it if the facts differ. Keep what is read, the Session claim rule and the Agent home rules exactly as ADR-0016, ADR-0021 and ADR-0002 define them. Three adapters make the seam real; do not add speculative methods.

Files: src/agent.rs, src/agent_logs.rs, src/agent_logs/*.rs, src/attention.rs, src/routing/policy.rs, src/session_attribution.rs, GLOSSARY.md.

Done when: no match on the agent is repeated for log reading, the tests for all three agents pass at the new interface, and Token usage, request analysis and attention results are unchanged.
```

## 5. Split `run_gate.rs` by job

Prerequisite: none.

- [ ] Option and request parsing, local gate evaluation, changed-file check selection and CI evaluation are separate modules behind one gate interface
- [ ] Stage-exit knowledge is in one place: `run_gate::accepted_exit` and `run/stage_exit.rs` no longer split it
- [ ] "Check" (the Stage) and a named check that reports Findings are named differently in the code and in [GLOSSARY.md](../../GLOSSARY.md)
- [ ] Tests that call private helpers (`changed_paths`, `tickets_verified`) go through the gate interface
- [ ] `scripts/verify.sh` passes
- [ ] PR merged into `develop`

Run / PR / verification: —

```text
Split run_gate.rs by job and bring stage-exit knowledge together.

Problem: run_gate.rs is about 1974 lines (about 800 of them tests) and mixes four jobs: request and option parsing (parse_options, parse_advance_options, AdvanceRequest), local gate evaluation (assess_local, evaluate_gate, ticket verification), changed-file check selection (changed_paths, select_checks) and CI evaluation (ci_passed, which takes &dyn PrHost). It exposes 12 pub(crate) items. Stage-exit knowledge is split between run_gate::accepted_exit and run/stage_exit.rs (about 852 lines). server.rs is the only caller of record, accepted_exit and AdvanceRequest. ADR-0001 (the CLI owns stage sequencing and gates) and ADR-0019 (stage-exit evidence) must keep holding. The word "check" means both the Check Stage and a named check that reports Findings.

Goal: separate the four jobs behind a small gate interface, bring accepted_exit and stage_exit together, and make the tests go through the interface. Do not split just for size. The dashboard API, gate results, stage exit records and error messages must not change.

Files: src/run_gate.rs, src/run/stage_exit.rs, src/run_check.rs, src/server.rs, src/main.rs, GLOSSARY.md.

Done when: the gate interface is smaller than today's 12 items or the extra items are justified, stage-exit knowledge has one home, and every existing gate and server test still passes.
```

## 6. Host URL and origin parsing in one place

Prerequisite: Step 1 merged.

- [ ] Remote and URL parsing for GitHub and Azure Repos lives in `pr_host`; `knowledge.rs` uses it instead of its own `github_repo`
- [ ] `pr_host` no longer depends on `doctor` for tool lookup (`doctor::az_program`)
- [ ] GitHub and Azure DevOps keep the same support
- [ ] `scripts/verify.sh` passes
- [ ] PR merged into `develop`

Run / PR / verification: —

```text
Consolidate host URL and origin parsing behind the pull request host seam.

Problem: pr_host has a real seam (trait Adapter with github.rs and azure.rs, Hosts<T> routing, trait Tools, FakeHost) and it should stay. But origin and URL parsing is split: knowledge.rs imports AzureRepo and encode_segment from pr_host and has its own github_repo parser (around line 251); pr_host::Origin and AzureRepo::from_remote do related work. pr_host.rs (around line 77) calls doctor::az_program, so the adapter depends on a diagnostics module for tool lookup, which points the wrong way.

Goal: one place parses remotes and pull request URLs for both hosts, with knowledge as a caller; the tool lookup helper moves out of doctor to where both doctor and pr_host can depend on it. Keep GitHub Enterprise and Azure Repos behavior the same. Do not change the seam itself.

Files: src/pr_host.rs, src/pr_host/github.rs, src/pr_host/azure.rs, src/knowledge.rs, src/knowledge_tests.rs, src/doctor.rs, src/pr_host/tests.rs.

Done when: there is one remote parser per host, no dependency from pr_host to doctor, and tests cover GitHub and Azure remotes the same way.
```

## 7. Group the root `run_*` and `pr_*` files

Prerequisite: Steps 2, 5 and 6 merged. Check `src/server.rs` first and only move files when the new grouping is clearly easier to navigate.

- [ ] The command implementations `run_begin`, `run_check`, `run_cleanup`, `run_gate`, `run_next` and the `pr_*` files sit in directories by concern, not flat in `src/`
- [ ] `server.rs` route handlers hold no orchestration that belongs in a module (for example the pr-status handler and the background refresh)
- [ ] `scripts/verify.sh` passes
- [ ] PR merged into `develop`

Run / PR / verification: —

```text
Group the flat run_* and pr_* modules and move orchestration out of server routes.

Problem: src/ has 45 modules declared in main.rs. run_begin, run_check, run_cleanup, run_gate, run_next, inspect and pr_open / pr_sync / pr_wait / pr_state sit flat beside run.rs, which has 9 submodules. server.rs (about 1482 lines, 52 functions) appears to hold orchestration in its routes (the pr-status handler, the background refresh around lines 960-1008 and 1342). This was only sampled; read it first.

Goal: group files by concern so the layout shows how a run moves through the code, and move route orchestration into modules with tests at their interface. Do not rename public behavior. Skip any move that does not clearly help a reader.

Files: src/main.rs (module list), src/run*.rs, src/pr_*.rs, src/inspect.rs, src/server.rs, src/server/.

Done when: the layout is easier to navigate, no behavior changed, and tests moved with their modules.
```

## 8. Move test helpers out of `main.rs`

Prerequisite: Step 2 merged.

- [ ] `new_run`, `advance_to` and `force_advance` (and similar helpers) live in a shared `test_support` module
- [ ] `main.rs` has no `pub(crate)` items that exist only for tests
- [ ] Tests that call private items of another module use that module's interface where one exists
- [ ] `scripts/verify.sh` passes
- [ ] PR merged into `develop`

Run / PR / verification: —

```text
Move test-only helpers out of main.rs.

Problem: new_run, advance_to and force_advance (src/main.rs around lines 1017-1080) are pub(crate) only because tests in other modules need them. 51 files use use super::* in their tests and so reach private items; 8 modules include a sibling xxx_tests.rs by path.

Goal: put the shared helpers in a test_support module and remove test-only items from main.rs. Fix tests that reach past an interface only where that is cheap; do not rewrite tests wholesale.

Files: src/main.rs, the tests that use these helpers.

Done when: main.rs has no test-only pub(crate) items and all tests pass.
```
