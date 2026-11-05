## Fixed

- `/factory` and `/factory-balanced` now report a run as `blocked`, with the reason "Decision needed on remaining Check findings", while they wait for the user to choose between another Rework, finishing with the findings listed, or stopping. The block clears when work resumes. Findings within the two-Rework allowance are still fixed without asking.
