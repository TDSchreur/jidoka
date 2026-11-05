# Humanizer integration plan

## Goal

Add [blader/humanizer](https://github.com/blader/humanizer) to the repository and the installed factory skill bundle. Use it when writing or editing prose for human readers, including PR descriptions, documentation, issues, specs, tickets, changelogs, and substantial explanations.

Apply it as an editing pass on new or changed prose. Preserve technical meaning, evidence, required structure, and the author's voice. Do not rewrite existing documentation unrelated to the task.

This document plans a future factory run; it does not start one.

## Factory prompt checklist

Use `/factory-balanced` for all three tickets: the scope is defined, and implementation needs behavior tests and a review pass. Run these prompts in order. Each prompt starts a separate factory run and PR. Merge each PR and complete the required local update before starting the next prompt, so its worktree includes the dependency. Include required documentation and changelog updates in each PR; the final run checks the complete integration.

- [ ] **Ticket 1: Vendor and bundle Humanizer.**

  ```text
  /factory-balanced Implement Ticket 1 in humanizer-plan.md: vendor blader/humanizer at a reviewed, pinned upstream commit and bundle it with factory install. Preserve attribution and its MIT license, fix installer license selection, and cover installed content, correct licensing, replacement, and supported agent layouts. Update skills/README.md and add the required changelog fragment in this PR. Follow the plan's applicable acceptance criteria and run scripts/verify.sh. Leave writing workflow changes for Ticket 2.
  ```

- [ ] **Ticket 2: Route human-facing writing through Humanizer.** Depends on Ticket 1.

  ```text
  /factory-balanced Implement Ticket 2 in humanizer-plan.md using the bundled Humanizer from the merged Ticket 1. Require it for human-facing prose through AGENTS.md, the shared factory procedures, and the pr skill. Cover all three factory workflows and standalone repository writing. Use embedded or file mode as appropriate, preserve required structure, facts, qualifications, evidence, and technical content, and keep bundled instructions project-agnostic. Update affected documentation and the changelog in this PR, verify skill resolution in checkout and installed layouts, and run scripts/verify.sh.
  ```

- [ ] **Ticket 3: Validate the complete integration and finish documentation.** Depends on Tickets 1 and 2.

  ```text
  /factory-balanced Implement Ticket 3 in humanizer-plan.md after Tickets 1 and 2 have merged. Check every acceptance criterion against the complete integration, finish any missing README and skills documentation, and add useful integration checks using existing test conventions. Exercise a PR description and Markdown document edit, checking prose quality, required sections, factual completeness, and unchanged technical content. Confirm routing in all three factory workflows and standalone writing tasks. Fix gaps, add a changelog fragment if this run changes user-visible behavior, run scripts/verify.sh, and include concrete validation evidence in the PR.
  ```

## Define

1. Start `/factory-balanced` using its normal run reporting and workspace procedures. Use a fresh worktree and target `develop`, as required by this repository.
2. Use this plan and the applicable acceptance criteria as the implementation brief. Reuse them without extra interviews or ticket breakdowns unless an ambiguity requires one. Follow tracker setup only when reading or publishing specs or tickets in the configured GitHub Issues tracker.
3. Inspect the upstream skill at a specific commit before copying it. Confirm its frontmatter, supporting-file requirements, MIT license, and embedded and file output modes. Record the full commit hash for reproducibility.
4. Inspect current skill discovery, bundling, license installation, and writing workflows. Read the repository's skill authoring, Rust, reliable testing, Markdown, and documentation guides where applicable.

### Acceptance criteria

- Humanizer is vendored with its upstream commit, attribution, and correct license recorded.
- `factory install` includes Humanizer for each supported agent and installs the Humanizer license without substituting another upstream project's license.
- Repository instructions require Humanizer for human-facing prose, including work outside a factory run.
- Installed factory workflows apply Humanizer to human-facing prose throughout Define, Build, Check, and Finish, without depending on this repository's `AGENTS.md`.
- The `pr` skill applies Humanizer before returning the finished description, preserving its required sections and evidence.
- Embedded writing tasks return only the finished prose. File edits preserve code, inline code, commands, paths, metadata, data, and link targets.
- Editing preserves facts, qualifications, domain terminology, and technical requirements. It does not invent details or remove required notices or actionable review findings.
- Bundled skills remain project-agnostic, and the project's verification passes.

## Build

Implement these tickets sequentially. Implement first, then add or adjust behavior tests under the balanced workflow. Run `scripts/verify.sh` before completing each ticket.

### Ticket 1: Vendor and bundle Humanizer

1. Copy the reviewed upstream skill and any required supporting files into `skills/humanizer/`. Prefer an unchanged upstream copy; document any necessary adaptation.
2. Preserve the upstream license under `third-party/` and record the source and pinned commit in `skills/README.md`.
3. Add Humanizer and its license to `build.rs`.
4. Update license selection in `src/install.rs` so Humanizer receives its own license. The current fallback assigns the mattpocock license to most upstream skills and must not apply here.
5. Extend installer coverage to verify the installed skill's content and correct license, including replacement of an existing installation and coverage through the supported agent layouts. Reuse existing test helpers and cases where practical.

### Ticket 2: Route human-facing writing through Humanizer

1. Add a concise rule to `AGENTS.md` covering new and edited human-facing prose. Link to `skills/humanizer/SKILL.md` so agents can load it directly in this checkout.
2. Add a shared writing procedure to `.agents/skills/factory-procedures/SKILL.md`, making it apply throughout all three factory workflows. Use the existing checkout and installed-skill resolution conventions.
3. Update `skills/pr/SKILL.md` to invoke Humanizer in embedded mode after drafting. Keep the required PR template, evidence, and remaining review findings intact.
4. Specify that Humanizer edits prose within the calling task's scope and output format. Use embedded mode for composed artifacts and file mode for document edits; do not emit intermediate drafts or critiques into PRs or documentation.
5. Keep project-specific paths, branch names, and verification commands in repository instructions. Bundled skill instructions must use generic language.
6. Check that upstream formatting advice cannot remove required headings, evidence labels, qualifications, or domain terms. Resolve any conflict with a short integration instruction; avoid duplicating the upstream pattern list.

### Ticket 3: Document and validate the behavior

1. Update `skills/README.md` with Humanizer's purpose, installation, provenance, and any local adaptations. Update `README.md` to explain its role in the factory workflow.
2. Add a `changelog.d/` fragment describing the bundled skill and writing behavior.
3. Add focused checks using existing test conventions for bundle inclusion and writing integration where useful. Avoid asserting subjective prose quality or copying whole instruction paragraphs into tests.
4. Manually exercise a PR description and a Markdown documentation edit. Verify that the final output is concise, factual, and complete, required PR sections survive, and technical content stays unchanged.
5. Confirm that all three factory workflows read the shared writing procedure and that standalone PR and repository documentation tasks can find Humanizer.

## Check

1. Inspect the final diff for scope, upstream attribution, license correctness, valid skill frontmatter, and working file references.
2. Search changed bundled skills for repository-specific names and conventions, as required by `AGENTS.md`.
3. Run `scripts/verify.sh`, fixing failures and missing required tooling. It includes the project's formatting, lint, and test checks.
4. Follow `/factory-balanced` review policy: commit the verified implementation, pin the reviewed commit, and run one code review against the recorded base commit and this plan. Run security review when the diff meets the workflow's criteria, including file handling in the installer. Resolve findings using the workflow's Rework procedure.
5. Treat writing quality as an agent-guided behavior supported by concrete examples. Automated checks can verify packaging and routing; they cannot guarantee every future passage reads naturally.

## Finish

1. Use the updated `pr` skill and Humanizer to write the PR description. Include verification evidence and any unresolved findings the user has chosen to accept.
2. Verify the final committed state under the workflow's rules, open the PR against `develop`, and monitor CI through completion.
3. After the PR merges, run `scripts/update-local.sh` from the main checkout, as required by `AGENTS.md`, to rebuild and reinstall the command and skills and restart the dashboard.
4. Confirm the local installation contains Humanizer with its correct license and that the writing instructions resolve it.
