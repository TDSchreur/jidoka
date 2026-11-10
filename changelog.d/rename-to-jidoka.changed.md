## Changed

- Breaking: the product is now called Jidoka — the AI software factory. The old names stop working and nothing is migrated, so runs recorded under the old data folder do not show up and you start fresh:
  - The command `factory` is now `jidoka` (for example `jidoka run begin`, `jidoka dashboard`, `jidoka install`). Delete the old `factory` binary manually.
  - The environment variables `FACTORY_*` are now `JIDOKA_*` (`JIDOKA_AGENT`, `JIDOKA_CLAUDE_CONFIG_DIRS`, `JIDOKA_USAGE` and so on).
  - The skills `factory` and `factory-*` are now `jidoka` and `jidoka-*` (for example `/jidoka`, `/jidoka-balanced`, `/jidoka-procedures`). `jidoka install` removes the old skill folders.
  - The data folder `tds-factory` and the config folder `tds-factory` (with `model-routing.json`) are now `jidoka`. Copy `model-routing.json` over by hand to keep your routing policy.
  - The git settings `factory.*` are now `jidoka.*`, and the dashboard token header `X-Factory-Token` is now `X-Jidoka-Token`.
  - The release downloads are now `jidoka-linux-x86_64` and `jidoka-windows-x86_64.exe`, and the repository is `TDSchreur/jidoka`.
