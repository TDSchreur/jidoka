## Changed

- `factory run requests` reads the commands agents really write: `R=$(cat file) && cd dir && factory run update $R ...` is a report and not chained work, command substitutions and filters after a pipe no longer leave stray commands behind, the commands inside a substitution are classified, a run id held in a variable set on the same line claims the run, and the request that starts a run counts as workspace work.
