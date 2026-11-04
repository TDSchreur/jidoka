---
applyTo: '.github/workflows/*.yml,.github/workflows/*.yaml'
description: 'Structure, security, caching, testing and deployment practices for GitHub Actions workflows.'
---

# GitHub Actions CI/CD Best Practices

Apply these rules when you write or review a workflow. Prefer secure and reliable over clever. This repository's workflows (`.github/workflows/ci.yml` and `release.yml`) show the house style; match them.

## Structure

- Give the workflow and every job a clear `name`. Make `on` triggers as narrow as the purpose allows (branch and path filters; `workflow_dispatch` for manual runs).
- Set `permissions: contents: read` at the workflow level and raise it per job only where a job needs more.
- Use `concurrency` (`group: ${{ github.workflow }}-${{ github.ref }}`) to cancel superseded runs. Do not cancel deployments or releases midway.
- Set `timeout-minutes` on every job so a hung job cannot run for six hours.
- Express order with `needs`. Pass data between jobs with `outputs`, and between workflows with `workflow_call` (reusable workflows) for patterns used more than once.
- Use `if` for conditional jobs and steps. Use `env` at the narrowest scope that works.
- Keep `run` steps short. Move anything longer than a few lines into a script in `scripts/` so it can be tested and linted (ShellCheck, PSScriptAnalyzer).
- Lint workflows with `actionlint`.

## Security

- **Pin every `uses:` to a full commit SHA** with the version in a comment (`actions/checkout@<sha> # v7`). Tags and branches are mutable and can be redirected to malicious code. This matters most for third-party actions. Dependabot can keep the SHAs current.
- Set `persist-credentials: false` on `actions/checkout` unless a later step pushes.
- Read secrets only through the `secrets` context, never hardcode them and never echo them (masking is not a guarantee). Scope secrets to an `environment` when only one deployment needs them. Prefer short-lived credentials.
- Use OpenID Connect (`permissions: id-token: write`) for cloud authentication instead of stored long-lived keys. Restrict the cloud role to the repository, branch or environment.
- Treat event data as untrusted input: PR titles, branch names, issue bodies and commit messages. Never interpolate `${{ github.event.* }}` directly into a `run:` script (script injection). Pass it through an `env:` variable and quote it in the script.
- Be careful with `pull_request_target` and `workflow_run`: they run with write access and secrets. Never check out and run code from a fork's PR in them.
- Run dependency review (`actions/dependency-review-action`) and a SAST scan (for example CodeQL) on PRs, with critical findings blocking the merge. Enable secret scanning and push protection on the repository.
- For container images, scan them and sign them (Cosign) when you publish them.
- Self-hosted runners: do not use them for public repositories, run them ephemeral and isolated, and restrict their network access.

## Performance

- Cache dependencies with the setup action's built-in cache (`actions/setup-node` with `cache: npm`) or `actions/cache` with a key built from `hashFiles('**/package-lock.json')` and sensible `restore-keys`. Never cache secrets or build output that depends on untrusted input.
- Use `strategy.matrix` for operating systems and language versions, with `fail-fast: false` when you want every result. Use `include` and `exclude` to trim combinations.
- Use shallow checkouts (`fetch-depth: 1`, the default) unless the job needs history (changelogs, `git describe`, merge-base).
- Pass files between jobs with `actions/upload-artifact` and `actions/download-artifact` instead of rebuilding. Set `retention-days`.
- Start the fast, cheap jobs (lint, unit tests) first and make slow jobs depend on them.

## Testing

- Run unit tests in their own early job. Run integration tests after them, with `services:` containers for dependencies. Run end-to-end tests against a staging or preview environment.
- Run performance and load tests only where they have a threshold that fails the job.
- Publish test reports and coverage as artifacts or Checks annotations so a failure can be read without opening the raw log. Enforce a coverage minimum where the project has one.
- A flaky test is a bug in the test: find the cause. Do not add retries or longer timeouts to hide it.
- One aggregate job (this repo's `ci-ok`) that `needs` every other job gives the branch ruleset a single required check. Add each new job to its `needs`.

## Deployment

- Use GitHub `environment` rules for staging and production: required reviewers for production, branch restrictions, environment-scoped secrets.
- Deploy to staging first, run smoke tests, then promote the same artifact to production. Do not rebuild for production.
- Choose the strategy that fits the risk: rolling for low risk, blue/green for instant switch and rollback, canary for gradual exposure.
- Every deployment needs a post-deployment health check and a tested rollback (redeploy the previous artifact, `kubectl rollout undo`, revert the release). Keep the previous artifact available.

## Review checklist

- [ ] Clear names; narrow triggers; `concurrency` set where runs can overlap.
- [ ] Top-level `permissions` is `contents: read`; job overrides are minimal and explained.
- [ ] Every `uses:` is pinned to a full SHA with a version comment; `persist-credentials: false` where nothing pushes.
- [ ] No secret in the file, a log or an artifact; no `${{ github.event.* }}` inside `run:`.
- [ ] OIDC instead of long-lived cloud keys; environments protect production.
- [ ] `timeout-minutes` on every job; caching keyed on the lockfile; artifacts have `retention-days`.
- [ ] Tests run in stages (unit, integration, end-to-end) and publish reports; the aggregate job includes the new job.
- [ ] Deployments have a health check and a rollback; `actionlint` passes.

## Troubleshooting

| Symptom                                       | Check                                                                                                                                                                          |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Workflow does not trigger                     | `on` event, branch and path filters, workflow file on the branch you pushed to, YAML syntax (run `actionlint`), `[skip ci]` in the message, a disabled workflow                |
| Job or step skipped                           | `if:` expression, `needs` that failed or were skipped, `matrix` `exclude`                                                                                                      |
| `Resource not accessible by integration`      | The job's `permissions` (a missing scope, or a fork PR token that is read-only), environment protection, a secret that is not available to forks                               |
| Cache miss or `Cache not found`               | Key changed (lockfile changed), cache is scoped to a branch and its base, cache evicted (10 GB limit, 7 days unused), wrong `path`                                             |
| Job times out or runs long                    | Set `timeout-minutes`, profile steps, add caching, split into parallel jobs, look for steps that wait for input                                                                |
| Passes locally, fails in CI                   | Different OS or tool version (pin them), missing environment variable or service, timing assumptions, shared state between tests, file-system case sensitivity or line endings |
| Deployment succeeds but the application fails | Environment variables and secrets in the target, migrations, health-check endpoint, config drift; roll back first, then investigate                                            |

When you debug, read the failing step's log first, then re-run with debug logging (`ACTIONS_STEP_DEBUG=true` as a repository secret or variable). Reproduce the failing command locally before you change the workflow.
