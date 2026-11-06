## Changed

- `factory run requests` reads commands the way agents write them. `R=$(cat file) && cd dir && factory run update $R ...` is a report, not chained work. Command substitutions are one word of their command, and the commands inside them are classified. A run id held in a variable set to a literal on the same line claims the run. The request that starts a run counts as workspace work.
