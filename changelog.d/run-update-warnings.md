## Added

- `factory run update` warns on stderr, without failing, when a stage is set to done that was never entered, or when a Full run finishes Build or starts Check with no spec. The server's update response lists the warnings.
