#!/bin/sh
# Update this checkout to the latest develop, rebuild jidoka, reinstall the `jidoka`
# command and its skills, and restart the dashboard.
set -eu

# Turns a choice such as "1,3", "claude copilot" or "all" into "codex,copilot".
parse_choice() (
  set -f
  chosen=''
  for token in $(printf '%s' "$1" | tr ',' ' '); do
    case "$token" in
      1 | codex) names='codex' ;;
      2 | claude) names='claude' ;;
      3 | copilot) names='copilot' ;;
      all) names='codex claude copilot' ;;
      *) return 1 ;;
    esac
    for name in $names; do
      case " $chosen " in
        *" $name "*) ;;
        *) chosen="$chosen $name" ;;
      esac
    done
  done
  [ -n "$chosen" ] || return 1
  printf '%s\n' "$chosen" | sed 's/^ //; s/ /,/g'
)

# Prints the chosen agents, asking on the terminal unless JIDOKA_AGENT is set.
choose_agents() {
  if [ -n "${JIDOKA_AGENT:-}" ]; then
    parse_choice "$JIDOKA_AGENT" || { echo "JIDOKA_AGENT must list codex, claude, or copilot, separated by commas." >&2; return 1; }
    return 0
  fi
  # The script itself may be piped into sh, so ask on the terminal instead of stdin.
  if ! (: </dev/tty) 2>/dev/null; then
    echo 'No terminal to ask which agents to install for. Set JIDOKA_AGENT, for example JIDOKA_AGENT=codex,claude.' >&2
    return 1
  fi
  while :; do
    printf '%s' 'Install the Jidoka skills for which agents?
  1) codex
  2) claude
  3) copilot
Enter one or more numbers or names separated by commas, or "all": ' >/dev/tty
    read -r answer </dev/tty || { echo 'No answer given.' >&2; return 1; }
    parse_choice "$answer" && return 0
    echo 'Invalid choice. Try again.' >/dev/tty
  done
}

# Stops any running `jidoka dashboard`; run data lives on disk, so nothing is lost.
stop_dashboard() {
  pattern='(^|/)jidoka dashboard$'
  pkill -f "$pattern" || return 0
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -f "$pattern" >/dev/null || return 0
    sleep 0.5
  done
  echo 'The old dashboard did not stop.' >&2
  return 1
}

start_dashboard() {
  log_dir="${XDG_STATE_HOME:-$HOME/.local/state}/jidoka"
  mkdir -p "$log_dir"
  nohup "$1" dashboard >"$log_dir/dashboard.log" 2>&1 &
  pid=$!
  sleep 1
  kill -0 "$pid" 2>/dev/null || { echo "Dashboard failed to start; see $log_dir/dashboard.log" >&2; return 1; }
  echo "Dashboard restarted (pid $pid, log $log_dir/dashboard.log)."
}

# Restores the changes stashed by main, also when a later step failed.
restore_changes() {
  trap - EXIT
  if git stash pop --index >/dev/null; then
    echo 'Restored local changes.'
  else
    echo 'Could not restore local changes cleanly; they remain in the stash (git stash list).' >&2
  fi
}

# Installs the skills, passing each entry of JIDOKA_CLAUDE_CONFIG_DIRS (separated by colons) as
# --config-dir. The positional parameters serve as an array, so paths with spaces stay intact.
install_skills() {
  binary=$1
  agents=$2
  dirs=${JIDOKA_CLAUDE_CONFIG_DIRS:-}
  set -- install --agent "$agents"
  old_ifs=$IFS
  IFS=:
  set -f
  for dir in $dirs; do
    if [ -n "$dir" ]; then set -- "$@" --config-dir "$dir"; fi
  done
  set +f
  IFS=$old_ifs
  "$binary" "$@"
}

main() {
  command -v pkill >/dev/null || { echo 'pkill (procps) is required.' >&2; exit 1; }
  agents=$(choose_agents) || exit 1
  root=$(git rev-parse --show-toplevel)
  cd "$root"
  if [ -n "$(git status --porcelain)" ]; then
    git stash push --include-untracked --message 'update-local.sh autostash' >/dev/null
    echo 'Stashed local changes; they are restored when the script ends.'
    trap restore_changes EXIT
    trap 'exit 1' HUP INT TERM
  fi
  git fetch origin develop
  git checkout develop
  git pull --ff-only origin develop
  cargo build --release --locked
  cargo install --path . --locked --force
  command -v jidoka >/dev/null 2>&1 || echo "Add ${CARGO_HOME:-$HOME/.cargo}/bin to PATH so the Jidoka command resolves." >&2
  install_skills "$root/target/release/jidoka" "$agents"
  "$root/target/release/jidoka" --version
  stop_dashboard
  start_dashboard "$root/target/release/jidoka"
}
main "$@"
