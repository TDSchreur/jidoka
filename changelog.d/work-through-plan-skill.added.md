## Added

- Add the `jidoka-work-through-plan` skill. Give it a plan file and it hands each open step to a fresh agent, unattended, until that step's pull request is merged, then records the result in the plan and moves to the next step. It stops when every step is done or a step is blocked.
