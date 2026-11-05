#!/bin/sh
# Update this checkout to the latest develop, rebuild factory, reinstall its skills
# and restart the dashboard.
set -eu

# Stops any running `factory dashboard`; run data lives on disk, so nothing is lost.
stop_dashboard() {
  pattern='(^|/)factory dashboard$'
  pkill -f "$pattern" || return 0
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -f "$pattern" >/dev/null || return 0
    sleep 0.5
  done
  echo 'The old dashboard did not stop.' >&2
  return 1
}

start_dashboard() {
  log_dir="${XDG_STATE_HOME:-$HOME/.local/state}/tds-factory"
  mkdir -p "$log_dir"
  nohup "$1" dashboard >"$log_dir/dashboard.log" 2>&1 &
  pid=$!
  sleep 1
  kill -0 "$pid" 2>/dev/null || { echo "Dashboard failed to start; see $log_dir/dashboard.log" >&2; return 1; }
  echo "Dashboard restarted (pid $pid, log $log_dir/dashboard.log)."
}

main() {
  command -v pkill >/dev/null || { echo 'pkill (procps) is required.' >&2; exit 1; }
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
  stop_dashboard
  start_dashboard "$root/target/release/factory"
}
main "$@"
