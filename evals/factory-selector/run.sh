#!/usr/bin/env bash
# Asks the installed `factory` selector which workflow fits each case in cases.json, a few
# times per case, and prints how often it chose an accepted mode. See README.md.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: run.sh [--runs N] [--model MODEL] [--skills-dir DIR] [--case ID]...

  --runs N          Ask N times per case (default 3).
  --model MODEL     Model for claude -p (default: the Claude Code default).
  --skills-dir DIR  Installed Claude Code skills (default: $CLAUDE_CONFIG_DIR/skills or ~/.claude/skills).
  --case ID         Run only this case; repeat for several.

Exit status: 0 every case passed, 3 some answers were wrong, 1 usage or tool error,
2 a factory run or a worktree appeared.
EOF
}

die() {
  printf 'run.sh: %s\n' "$1" >&2
  exit 1
}

# The throwaway directory; global so the EXIT trap can remove it.
work=''
cleanup() {
  [ -z "$work" ] || rm -rf -- "$work"
}

# Prints the mode named on the last `MODE:` line of an answer, or nothing.
# Accepts Markdown around it, such as **MODE:** full or `MODE: full`.
mode_of() {
  # shellcheck disable=SC2016 # the backtick is a literal Markdown character.
  grep -Eio 'MODE[*`]*:[*`[:space:]]*(full|balanced|fast|none)([^a-z0-9-]|$)' |
    tail -n 1 | tr '[:upper:]' '[:lower:]' | grep -Eo '(full|balanced|fast|none)' | tail -n 1 || true
}

# Prints the IDs of every recorded factory run, one per line, sorted.
run_ids() {
  factory run stats --all | awk '$1 ~ /^[0-9]+$/ { print $1 }' | sort -u
}

# Builds the analysis-only prompt for one request.
prompt_for() {
  printf '/factory Which factory workflow would this request get if it were run for real, and why? Only answer the question: do not start a run, create a worktree or change any file now.\n\nRequest: %s\n\nEnd your answer with one final line that is exactly one of: MODE: full, MODE: balanced, MODE: fast, or MODE: none. Use MODE: none only when the request itself should not start a new run, for example because it continues an existing one. That this question is analysis only does not make the answer none.\n' "$1"
}

main() {
  local here runs=3 model='' skills_dir='' cases=()
  here=$(cd -- "$(dirname -- "$0")" && pwd)
  skills_dir="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/skills"

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --runs) [ "$#" -ge 2 ] || die '--runs needs a number'; runs=$2; shift 2 ;;
      --model) [ "$#" -ge 2 ] || die '--model needs a name'; model=$2; shift 2 ;;
      --skills-dir) [ "$#" -ge 2 ] || die '--skills-dir needs a directory'; skills_dir=$2; shift 2 ;;
      --case) [ "$#" -ge 2 ] || die '--case needs an id'; cases+=("$2"); shift 2 ;;
      -h | --help) usage; exit 0 ;;
      *) printf 'run.sh: unknown argument: %s\n' "$1" >&2; usage >&2; exit 1 ;;
    esac
  done
  case "$runs" in
    '' | *[!0-9]* | 0) die "--runs must be a positive number, not '$runs'" ;;
  esac

  for tool in claude jq factory git timeout awk comm; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required but not on PATH"
  done
  [ -f "$skills_dir/factory/SKILL.md" ] ||
    die "no factory skill in $skills_dir; install the skills with factory install --agent claude, or pass --skills-dir"

  local case_file="$here/cases.json"
  jq -e 'type == "array" and length > 0' "$case_file" >/dev/null || die "$case_file holds no cases"
  local ids
  if [ "${#cases[@]}" -gt 0 ]; then
    for id in "${cases[@]}"; do
      jq -e --arg id "$id" 'any(.[]; .id == $id)' "$case_file" >/dev/null || die "no case with id '$id'"
    done
    ids=$(printf '%s\n' "${cases[@]}")
  else
    ids=$(jq -r '.[].id' "$case_file")
  fi

  # Every claude call runs in a throwaway copy of the fixture repository.
  work=$(mktemp -d)
  trap cleanup EXIT
  trap 'exit 1' HUP INT TERM
  local repo="$work/repo"
  cp -R -- "$here/fixture" "$repo"
  git -C "$repo" init -q -b main
  git -C "$repo" add -A
  git -C "$repo" -c user.name=eval -c user.email=eval@example.invalid commit -q -m fixture

  local runs_before
  runs_before=$(run_ids) || die 'factory run stats --all failed'

  # Only reading tools exist in the session, `dontAsk` refuses anything not allowed below,
  # and the deny list wins over any broader allow rule in the user's own settings, so the
  # selector cannot start a run, create a worktree or change a file.
  local claude_args=(
    --output-format json
    --add-dir "$skills_dir"
    --permission-mode dontAsk
    --tools Read Grep Glob Bash
    --allowedTools 'Read' 'Grep' 'Glob' 'Bash(factory run inspect:*)' 'Bash(git status:*)' 'Bash(git log:*)'
    --disallowedTools 'Write' 'Edit' 'Bash(factory run begin:*)' 'Bash(factory run start:*)' 'Bash(factory run update:*)' 'Bash(factory pr:*)' 'Bash(git worktree:*)' 'Bash(git commit:*)' 'Bash(git push:*)'
  )
  [ -z "$model" ] || claude_args+=(--model "$model")

  local total=0 passed=0 failed_cases=0 cost=0
  printf '%-26s %-7s %s\n' case passed 'modes seen'
  while IFS= read -r id; do
    local request accepted
    request=$(jq -r --arg id "$id" '.[] | select(.id == $id) | .request' "$case_file")
    accepted=$(jq -r --arg id "$id" '.[] | select(.id == $id) | .accepted | join(" ")' "$case_file")
    local ok=0 seen=() i
    for ((i = 1; i <= runs; i++)); do
      local out mode result status=0 err="$work/claude.err"
      out=$(cd -- "$repo" && timeout 600 claude -p "$(prompt_for "$request")" "${claude_args[@]}" </dev/null 2>"$err") || status=$?
      # A failed call often still prints JSON with the error in `result`; keep it.
      result=$(printf '%s' "$out" | jq -r '.result // ""' 2>/dev/null || printf '%s' "$out")
      cost=$(jq -rn --argjson sum "$cost" --arg out "$out" '$sum + (($out | fromjson? | .total_cost_usd) // 0)')
      mode=$(printf '%s\n' "$result" | mode_of)
      if [ -z "$mode" ]; then
        mode='?'
        if [ "$status" -eq 124 ]; then
          printf 'run.sh: %s run %d timed out after 600 seconds\n' "$id" "$i" >&2
        elif [ "$status" -ne 0 ]; then
          printf 'run.sh: %s run %d: claude exited with status %d; it said:\n' "$id" "$i" "$status" >&2
        else
          printf 'run.sh: %s run %d gave no MODE line; the answer ended:\n' "$id" "$i" >&2
        fi
        { tail -n 5 -- "$err"; printf '%s\n' "$result" | tail -n 3; } | sed 's/^/  /' >&2
      fi
      seen+=("$mode")
      total=$((total + 1))
      case " $accepted " in
        *" $mode "*) ok=$((ok + 1)); passed=$((passed + 1)) ;;
      esac
    done
    [ "$ok" -eq "$runs" ] || failed_cases=$((failed_cases + 1))
    printf '%-26s %-7s %s (accepted: %s)\n' "$id" "$ok/$runs" "${seen[*]}" "$accepted"
  done <<<"$ids"

  printf '\nOverall: %d/%d answers passed (%d%%); %d case(s) not always right; cost $%.2f\n' \
    "$passed" "$total" $((passed * 100 / total)) "$failed_cases" "$cost"

  # The prompts forbid side effects; check that none happened anyway.
  local runs_after new_runs worktrees
  runs_after=$(run_ids) || die 'factory run stats --all failed'
  new_runs=$(comm -13 <(printf '%s\n' "$runs_before") <(printf '%s\n' "$runs_after"))
  worktrees=$(git -C "$repo" worktree list --porcelain | grep -c '^worktree ' || true)
  if [ -n "$new_runs" ] || [ "$worktrees" -ne 1 ]; then
    [ -z "$new_runs" ] || printf 'run.sh: factory runs appeared during the eval: %s\n' "$(printf '%s' "$new_runs" | tr '\n' ' ')" >&2
    [ "$worktrees" -eq 1 ] || printf 'run.sh: the fixture repository has %d worktrees instead of 1\n' "$worktrees" >&2
    exit 2
  fi

  [ "$failed_cases" -eq 0 ] || exit 3
}

main "$@"
