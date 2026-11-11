## Added

- Record stage gate results with `jidoka run gate` and check them before moving with `jidoka run advance`.

## Changed

- Run storage now includes gate results. Existing runs are not migrated; finish or abandon in-flight runs before upgrading.
