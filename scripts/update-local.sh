#!/bin/sh
# Update this checkout to the latest develop, rebuild factory and reinstall its skills.
set -eu

main() {
  agent="${FACTORY_AGENT:-claude}"
  root=$(git rev-parse --show-toplevel)
  cd "$root"
  [ -z "$(git status --porcelain)" ] || { echo 'Working tree has changes; commit or stash them first.' >&2; exit 1; }
  git fetch origin develop
  git checkout develop
  git pull --ff-only origin develop
  cargo build --release --locked
  "$root/target/release/factory" install --agent "$agent"
  "$root/target/release/factory" --version
}
main "$@"
