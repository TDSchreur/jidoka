## Fixed

- `jidoka run update` no longer rejects free-text values that start with `--`, such as a `--finding` text that begins with an option name. A lone `--word` where a value belongs is still treated as a forgotten value, and the error now says what was found.
