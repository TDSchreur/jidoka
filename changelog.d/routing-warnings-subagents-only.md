## Fixed

- Model routing warnings and `jidoka run inspect` now judge only subagent usage. A Stage whose usage is all on the coordinator no longer gets a warning or a `MISMATCH`; `inspect` says it was kept on the coordinator, and a coordinator on the requested model no longer hides subagents that ran another model.
