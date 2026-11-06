# Migrate the dashboard server to Rocket

## Problem and outcome

TDS Factory's dashboard server currently implements HTTP parsing and routing itself. Replace that server with Rocket to make the local server easier to extend, while preserving the CLI, dashboard, run API, existing saved runs and intended local trust boundary. Include the existing uncommitted GitHub Actions instruction, `AGENTS.md`, and `.gitignore` edits in this work.

## Behavior

- `factory dashboard` serves the existing dashboard on `127.0.0.1:4311` on Linux and Windows. It reports startup failures and retains existing CLI help, install and run commands. An existing run record remains readable and writable across upgrade and restart.
- Preserve `GET /`, `GET /api/runs`, `POST /api/runs`, and `POST /api/runs/<id>` for the existing CLI and dashboard, including JSON field names, valid stage/status values, successful response codes and relevant invalid-input/not-found behavior. Do not require clients to change.
- Keep the existing local security boundaries: reject untrusted Host values, reject POST requests with an Origin header or without JSON content type, and limit request body size. The current server has no same-machine user authentication; do not imply otherwise. Reject malformed or oversized input without changing stored data.
- Rocket may serve multiple requests concurrently. Serialize each complete load-modify-save transaction so simultaneous creates and updates cannot lose existing runs, corrupt records, or reuse an ID.
- Use Rocket's routing, request parsing, JSON support and limits instead of implementing HTTP parsing. Keep the runtime and dependency choices suitable for the existing single-binary release on Linux and Windows. Preserve the user-facing data paths and on-disk JSON format.
- Add the copied GitHub Actions instruction as requested, link it from `AGENTS.md`, and ignore `.factory/`; retain the user's existing edits.

## Verification

Test at the HTTP boundary and through the CLI where practical: dashboard output, create/list/update, persisted data across restart or reload, invalid IDs, invalid JSON, content type, Origin, Host, oversized payload, and concurrent updates. Verify an existing CLI can talk to the new server. Use isolated directories, OS-assigned ports and bounded waits. Run the configured formatting, Clippy and test checks; run Windows checks where available and report those unavailable.

## Documents

Review and update applicable claims in `README.md`, `docs/specs/run-dashboard.md`, `docs/specs/run-dashboard-tickets.md`, `.agents/skills/factory-full/SKILL.md`, and the CLI help in `src/main.rs`. The currently requested `AGENTS.md`, `.gitignore`, and `.github/instructions/github-actions-ci-cd-best-practices.instructions.md` changes are also in scope. The sweep additionally listed `skills/rust/references/transform-over-mutate.md`, `skills/rust/type-design.md`, `skills/security-review/references/report-format.md`, and `skills/security-review/references/vuln-categories.md`; inspect hits and change them only if they describe this behavior.

## Estimate and scope decision

| Work | Estimated changed files | Estimated diff |
| --- | ---: | ---: |
| Define spec | 1 | 4 KB |
| Rocket server, dependency, tests and affected documentation | 3–5 | 24–35 KB |
| Existing instructions, agent guide and ignore changes | 3 | 9 KB |
| Total, excluding lockfiles | 7–9 | 37–48 KB |

The user chose to keep the requested changes together after being told the run may exceed the size limit. If it grows further during Build, follow the Factory size gate.

## Out of scope

Remote exposure, user authentication, new dashboard features, a new storage engine, and changes to release or installation workflows.
