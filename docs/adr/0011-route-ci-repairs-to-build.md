---
status: accepted
---

# Route CI repairs to Build

CI diagnosis and repair are implementation work even though they occur during Finish. Route them to the effective Build choice; PR text continues to use Finish. Keep the existing stage graph and coordinator behavior for Fast and small tasks. Rework raises known lower Build efforts to high, preserving stronger and unknown settings. Policies may offer optional complex choices, selected as run overrides with a recorded reason.

## Considered Options

- **Keep all Finish work on Finish.** Rejected because a lightweight model selected for PR text may be unsuitable for repairs.
- **Add a CI stage.** Rejected because it adds bookkeeping without changing the workflow.

## Consequences

- Existing policies load unchanged and do not acquire optional complex choices automatically.
- Usage is aggregated by stage, so Finish usage cannot prove which task used each model. Build usage cannot prove effort compliance across rework cycles.
- The run retains up to 32 distinct prior choices per stage when overrides change. This bounds the record without silently discarding assignments that explain aggregate usage; another distinct change is rejected when the limit is reached.
