#!/usr/bin/env bash
# Asks GitHub Copilot CLI, with the installed jidoka skills, what it would do next in each
# flow case in cases.json, and prints how often the answer was accepted. See README.md.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: run.sh [--runs N] [--model MODEL] [--skills-dir DIR] [--case ID]...

  --runs N          Ask N times per case (default 3).
  --model MODEL     Model for copilot -p (default: the Copilot default).
  --skills-dir DIR  Installed Copilot skills (default: $COPILOT_HOME/skills or ~/.copilot/skills).
  --case ID         Run only this case; repeat for several.

Exit status: 0 every case passed, 3 some answers were wrong, 1 usage or tool error.
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

# Prints the key on the last `ACTION:` line of an answer, or nothing.
action_of() {
  # shellcheck disable=SC2016 # the backtick is a literal Markdown character.
  { grep -Eio 'ACTION[*`]*:[*`[:space:]]*[a-z0-9-]+' || true; } |
    tail -n 1 | sed -E 's/^[^:]*:[*`[:space:]]*//' | tr '[:upper:]' '[:lower:]'
}

# Builds the analysis-only prompt for the case with id $1, skills in $2, cases in $3.
prompt_for() {
  jq -r --arg skills "$2" --arg id "$1" '.[] | select(.id == $id) |
    "Read \($skills)/jidoka-procedures/SKILL.md and the files under its references/ that apply, and follow them. Only answer the question: do not run any command or change any file.\n\nScenario: \(.scenario)\n\n\(.question) Choose exactly one:\n"
    + (.actions | to_entries | map("- \(.key): \(.value)") | join("\n"))
    + "\n\nEnd your answer with one final line that is exactly ACTION: <key>."' "$3"
}

main() {
  local here runs=3 model='' skills_dir='' cases=()
  here=$(cd -- "$(dirname -- "$0")" && pwd)
  skills_dir="${COPILOT_HOME:-$HOME/.copilot}/skills"

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

  for tool in copilot jq timeout; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required but not on PATH"
  done
  [ -f "$skills_dir/jidoka-procedures/SKILL.md" ] ||
    die "no jidoka-procedures skill in $skills_dir; install the skills with jidoka install --agent copilot, or pass --skills-dir"

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

  work=$(mktemp -d)
  trap cleanup EXIT
  trap 'exit 1' HUP INT TERM

  # Only reading tools exist in the session, so no command can run and no file can change.
  local copilot_args=(
    -s --no-ask-user
    --add-dir "$skills_dir"
    --available-tools view grep glob
    --allow-tool view grep glob
  )
  [ -z "$model" ] || copilot_args+=(--model "$model")

  local total=0 passed=0 failed_cases=0 unvalidated=0
  printf '%-26s %-7s %-11s %s\n' case passed status 'actions seen'
  while IFS= read -r id; do
    local accepted validated status
    accepted=$(jq -r --arg id "$id" '.[] | select(.id == $id) | .accepted | join(" ")' "$case_file")
    validated=$(jq -r --arg id "$id" '.[] | select(.id == $id) | .validated' "$case_file")
    status=validated
    if [ "$validated" != true ]; then
      status=UNVALIDATED
      unvalidated=$((unvalidated + 1))
    fi
    local ok=0 seen=() i
    for ((i = 1; i <= runs; i++)); do
      local out action rc=0 err="$work/copilot.err"
      out=$(cd -- "$work" && timeout 600 copilot -p "$(prompt_for "$id" "$skills_dir" "$case_file")" "${copilot_args[@]}" </dev/null 2>"$err") || rc=$?
      action=$(printf '%s\n' "$out" | action_of)
      if [ -z "$action" ]; then
        action='?'
        if [ "$rc" -eq 124 ]; then
          printf 'run.sh: %s run %d timed out after 600 seconds\n' "$id" "$i" >&2
        elif [ "$rc" -ne 0 ]; then
          printf 'run.sh: %s run %d: copilot exited with status %d; it said:\n' "$id" "$i" "$rc" >&2
        else
          printf 'run.sh: %s run %d gave no ACTION line; the answer ended:\n' "$id" "$i" >&2
        fi
        { tail -n 5 -- "$err"; printf '%s\n' "$out" | tail -n 3; } | sed 's/^/  /' >&2
      fi
      seen+=("$action")
      total=$((total + 1))
      case " $accepted " in
        *" $action "*) ok=$((ok + 1)); passed=$((passed + 1)) ;;
      esac
    done
    [ "$ok" -eq "$runs" ] || failed_cases=$((failed_cases + 1))
    printf '%-26s %-7s %-11s %s (accepted: %s)\n' "$id" "$ok/$runs" "$status" "${seen[*]}" "$accepted"
  done <<<"$ids"

  printf '\nOverall: %d/%d answers passed (%d%%); %d case(s) not always right; %d case(s) UNVALIDATED\n' \
    "$passed" "$total" $((passed * 100 / total)) "$failed_cases" "$unvalidated"
  [ "$failed_cases" -eq 0 ] || exit 3
}

main "$@"
