# Factory routing release validation

Validated the selector from merged commit `44cfe29` before preparing v0.6.0.

## Results

- The implementation PR passed Ubuntu and Windows CI. Automated tests cover legacy and canonical mode input, persistence, inspection, and replacement and reinstall of all four entry points for Codex, Claude, and Copilot.
- A live spelling-fix request selected Fast, loaded its installed workflow, recorded `factory-fast`, corrected the file, passed verification, and created a local commit.
- Continuation preserved the recorded executor for Full, Balanced, and Fast. A fixture stored with legacy `factory` was inspected as `factory-full` and resumed through the Full workflow.
- A fixture with no mode reported unknown context and stopped without guessing or creating a replacement run.
- An analysis-only authentication redesign request recommended Full without implementation or run creation.
- Persistent Balanced and Full sessions returned separate Standards and Spec reviewer responses. Full also completed a scoped security review. Initial ephemeral sessions hit a thread-store error; persistent retries succeeded. Command-line output omitted reviewer messages, so their results were verified in the persistent session records.

## Scope and limits

The live exercise used a disposable Git project, a local bare origin, and isolated dashboard data. Its temporary Factory build changed only the default dashboard port and address. It used installed workflow skills and a pre-provisioned dedicated worktree. The exercise stopped before external PR creation and CI. Fresh Balanced and Full feature implementations and direct entry-point bypass were not exercised end to end; the analysis-only routing matrix covered their selection rules.

No implementation defect requiring a release fix was established. Upgrade the command, dashboard, and installed skills together: older binaries may not read the new canonical `factory-full` mode. `/factory` now selects a workflow; `/factory-full` provides the former behavior.
