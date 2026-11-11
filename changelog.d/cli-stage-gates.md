## Added

- Record stage gate results with `jidoka run gate` and check them before moving with `jidoka run advance`.

## Changed

- Current-ticket references accept local paths and identifiers as well as HTTP(S) URLs; like listed ticket URLs, they no longer accept query strings or fragments.
- Run storage now includes gate results. Existing runs are not migrated; finish or abandon in-flight runs before upgrading.
