# Releasing TDS Factory

The [old project](https://github.com/TDSchreur/tds-factory) uses `develop` for daily work and its release branch for published code. This project follows the same flow, with `master` as its release branch.

1. Open ordinary pull requests against `develop` and squash-merge them after `ci-ok` passes. `develop` requires an up-to-date branch, so a pull request that falls behind after another one lands needs the base merged in and CI run again. The factory's agents do this themselves; for your own branches use the Update branch button on the pull request (the repository allows it).
2. When ready to release, update the version in `Cargo.toml` and `Cargo.lock` on `develop`, through a pull request. Choose a version whose `v<version>` tag does not already exist.
3. Open a pull request from `develop` to `master`. Once `ci-ok` passes, merge it with a merge commit so the branches retain their shared history.
4. The release workflow runs from the resulting `master` commit, builds and tests Linux and Windows binaries, and creates the matching version tag and GitHub Release. Release runs do not interrupt one another; GitHub keeps at most one pending run, replacing an older pending run if another starts. Build artifacts expire after seven days. Check the release has both binaries, both checksums, and both installer scripts. For a transient failure before tag creation, rerun the workflow on `master`; if the tag already exists, investigate before retrying.

Do not publish manually or push tags as a substitute for the release workflow. The installers need all release assets. A public one-line installer also requires the repository and release to be accessible to its users.

## Branch protection

The matching rules from the old project are captured in `.github/rulesets/`: `develop` requires a squash PR, signed commits, linear history, resolved review threads and a successful up-to-date `ci-ok`; `master` requires a merge-commit PR, signed commits, resolved review threads and `ci-ok`. Both prevent deletion and force pushes. Release tags cannot be updated or deleted except by the repository administrator.

These rulesets are active on GitHub. If they need to be recreated, apply each file and verify the rules in GitHub:

```sh
gh api --method POST repos/TDSchreur/tds-factory2/rulesets --input .github/rulesets/develop.json
gh api --method POST repos/TDSchreur/tds-factory2/rulesets --input .github/rulesets/master.json
gh api --method POST repos/TDSchreur/tds-factory2/rulesets --input .github/rulesets/release-tags.json
```
