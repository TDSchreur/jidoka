# Factory skill improvements

Static analysis of `factory`, `factory-balanced`, `factory-fast`, their supporting skills, and the README. These are proposed improvements; the checklist tracks implementation, not approval of every design choice.

## Assessment

The three modes have useful, distinct purposes. Keep all three and keep balanced as the recommended default.

| Mode | Purpose | Assessment |
| --- | --- | --- |
| `factory` | Thorough planning, test-first implementation, code and security review | Coherent, but its implementation skill needs substantial overrides |
| `factory-balanced` | Light planning, implementation followed by tests, code review | Sensible default; repeated verification makes it heavier than necessary |
| `factory-fast` | Minimal planning, selective tests, no Check stage | Appropriate for small changes; mandatory tracker setup adds avoidable friction |

The README mostly describes the workflows accurately. The largest problems are in the handoffs between skills and in duplicated operating instructions.

## Findings and recommendations

### 1. Ensure Check reviews the implementation

The workflows commit in Finish, after Check. However, [code-review](skills/code-review/SKILL.md) uses `git diff <fixed-point>...HEAD`, which excludes uncommitted changes. Following the instructions literally can produce an empty review or omit the latest implementation or Rework changes.

Commit completed work before Check, including Rework fixes, so every review has a precise snapshot. Alternatively, explicitly support reviewing working-tree changes and new files. Committing before Check is the preferred approach.

### 2. Simplify the test-first implementation skill

[implement-spec](skills/implement-spec/SKILL.md) prescribes parallel implementers, separate worktrees, merging, a draft PR, and another review. [factory](.agents/skills/factory/SKILL.md) overrides those instructions to require sequential implementation in one worktree.

The precedence rule resolves the conflict, but agents must load and disregard much of the implementation skill. Give the test-first implementation skill the same straightforward structure as the balanced and fast variants, while preserving TDD. Preserve upstream attribution and document local adaptations.

### 3. Consolidate shared operating instructions

Reporting, continuation, Workspace, setup recovery, and Finish are largely duplicated across the three workflows. Together, the workflow files contain approximately 5,400 words at the time of analysis.

Move shared procedures into one installed reference. Keep each workflow focused on its planning, testing, review, and verification policies. Ensure the reference is available both in this checkout and in every supported installation layout. Avoid introducing another orchestration framework.

### 4. Reduce redundant verification

Balanced requires full verification after every ticket, and all modes require it again on the final commit. Fast also permits reusing passing results, making its Finish wording ambiguous.

Prefer relevant checks during implementation and full verification on the final candidate. Repeat checks after changes that affect their coverage, and always honor stricter project requirements. Quality mode may retain full verification per ticket if that cost is intentional. Make the relationship between Build checks, pre-Check commits, and final verification explicit.

### 5. Make tracker setup conditional in lighter modes

Balanced and fast block without tracker setup even when no separate spec or tickets are needed. Fast explicitly permits proceeding directly from a clear request and conversational plan.

Require tracker configuration when publishing planning artifacts. Let clear, bounded requests proceed without tracker setup when no tracker operations are needed. Preserve the configured tracker when artifacts are needed; never guess one. Update readiness guidance to explain any mode-dependent requirements.

### 6. Preserve lightweight plans for continuation

Balanced and fast can keep the spec and acceptance criteria entirely in the conversation. Continuation relies on saved run context, but nothing explicitly requires preserving those criteria.

Save a concise plan and acceptance criteria in a durable location and record its reference before Build. This should not require a tracker or a heavyweight spec. Preserve the rule that run context is updated through the CLI, never by editing factory storage directly.

### 7. Report decision waits consistently

The reporting rule says outside input means `blocked`, but Check explicitly keeps a run `running` while waiting for the user to decide how to handle unresolved findings.

Use `blocked` while awaiting that decision and clear the blocking reason when work resumes. Preserve the choices to grant another Rework, finish with findings documented, or stop. Findings should trigger autonomous fixes within the existing Rework allowance; the decision wait is what blocks progress.

### 8. Correct and clarify the README

The comparison table and mode guidance are useful. Make these targeted changes:

- Replace ambiguous “one review pass” wording with “one initial code review, with follow-up reviews of fixes.”
- Extend the workflow diagram through CI watching and failure repair; opening a PR alone does not complete a run.
- Replace the diagram's hardcoded `develop` destination with the selected project base branch.
- Explain that fast still runs mandatory project verification and waits for CI. Its savings come from lighter planning, selective test writing, and skipping Check.
- Keep setup, continuation, verification, and shared-reference documentation aligned with the implemented improvements.

## Implementation checklist and factory prompts

Run these in order. Each prompt describes a separate change suitable for `/factory-balanced`. Later prompts should use the results of earlier changes instead of recreating them. Update this checklist as each change lands.

- [x] **1. Fix review coverage and commit ordering.**

  ```text
  /factory-balanced Fix the factory and factory-balanced Check handoff so code-review always reviews the complete implementation. Commit completed implementation and each Rework before Check, pin the review baseline and reviewed commit, and adjust Finish accordingly. Ensure follow-up reviews include the fixes and new files. Preserve the prohibition on amend and force-push for CI fixes. Update affected documentation, add a changelog fragment, and run the project's verification.
  ```

- [x] **2. Simplify test-first implementation.**

  ```text
  /factory-balanced Simplify the test-first implement-spec skill used by factory. Follow the structure of implement-spec-balanced and implement-spec-fast: sequential implementation in the run's existing worktree and branch, with TDD for each ticket. Remove requirements for implementer or merger subagents, extra worktrees, a draft PR, and a duplicate code review. Leave PR creation to the calling workflow. Preserve upstream attribution, document local adaptations in skills/README.md, update affected documentation, add a changelog fragment, and run the project's verification.
  ```

- [ ] **3. Extract shared workflow procedures.**

  ```text
  /factory-balanced Consolidate the duplicated operating procedures in factory, factory-balanced, and factory-fast into one shared installed reference. Preserve each mode's behavior and leave its planning, testing, review, and verification policy explicit in its own SKILL.md. Ensure references resolve in the repository and for Codex, Claude, and Copilot installations; adjust bundling only as needed. Keep bundled instructions project-agnostic. Update affected documentation, add a changelog fragment, and run the project's verification.
  ```

- [ ] **4. Remove unnecessary repeated verification.**

  ```text
  /factory-balanced Align verification instructions across the three factory workflows and implementation skills. For balanced and fast, run relevant checks during implementation and full verification on the final candidate; reuse passing results only while the checked code and relevant inputs remain unchanged. Always honor stricter project-required checks. Retain quality mode's full verification per ticket. Reconcile these rules with commits before Check and verification before push so the instructions do not demand redundant runs without a reason. Update documentation, add a changelog fragment, and run the project's verification.
  ```

- [ ] **5. Make lightweight runs independent of tracker setup and resumable.**

  ```text
  /factory-balanced Let factory-balanced and factory-fast handle clear, bounded requests without tracker setup when no tracker operations are needed. Preserve tracker requirements for publishing specs or tickets and never guess a tracker. Before Build, durably save the lightweight plan and acceptance criteria and record its reference through the existing run-context CLI so another session can continue without the original chat. Keep this lightweight and avoid new CLI features unless existing capabilities are insufficient. Align setup recovery and readiness guidance, update documentation, add a changelog fragment, and run the project's verification.
  ```

- [ ] **6. Make decision-wait status consistent.**

  ```text
  /factory-balanced Make factory and factory-balanced report blocked when unresolved Check findings require a user decision after the Rework allowance is exhausted or a finding cannot be resolved. Preserve autonomous fixes within the allowance and the existing choices to grant another Rework, finish with documented findings, or stop. Clear the blocking reason when work resumes. Update shared instructions and README wording together, add a changelog fragment, and run the project's verification.
  ```

- [ ] **7. Finish the README alignment pass.**

  ```text
  /factory-balanced Audit README.md and skills/README.md against the final factory, factory-balanced, factory-fast, and supporting skills. Clarify that balanced has one initial review plus reviews of Rework fixes. Extend the workflow diagram through CI watching and failure repair, label the PR destination as the selected base branch, and explain that fast still requires project verification and green CI. Check setup, continuation, shared-reference, and verification descriptions against the implemented behavior. Correct remaining discrepancies without expanding scope and run the project's verification.
  ```

## Completion criteria

- Every mode has clear planning, implementation, verification, review, and completion rules.
- Check reviews the actual implementation and all subsequent fixes.
- Shared instructions resolve in source and installed layouts without repository-specific conventions leaking into bundled skills.
- Lightweight runs preserve enough context to continue in a new session.
- README descriptions and diagrams match the shipped workflows.
- Each implementation change includes appropriate documentation, a changelog fragment when user-visible, and successful project verification.
