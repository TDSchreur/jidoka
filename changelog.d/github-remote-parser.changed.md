## Changed

- `jidoka pr open`, `jidoka doctor` and knowledge repo spec links recognize a github.com remote the same way. A github.com remote with an unusual port, such as `ssh://git@github.com:2222/owner/repo.git`, now works with `pr open` and `doctor`, and a `git://` knowledge repo origin no longer gets spec links (GitHub dropped that protocol in 2022). See ADR-0025.
