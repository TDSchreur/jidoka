## Changed

- The model routing procedure moved out of the shared `factory-procedures` skill into `factory-procedures/references/model-routing.md`. `/factory-full` and `/factory-balanced` read it at the start of every Check pass, so runs no longer load it up front, and `/factory-fast`, which has no Check, never does. What it says is unchanged.
