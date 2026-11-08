#!/usr/bin/env python3
"""Evaluate a PR skill against a fixed fixture; see README.md."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent


def check(answer, fixture):
    failures = []
    if not answer.startswith("## Summary\n"):
        failures.append("answer must start with ## Summary")
    headings = re.findall(r"(?m)^## (.+?)\s*$", answer)
    for heading in ("Summary", "Evidence", "Merge Danger", "Remaining review findings"):
        if heading not in headings:
            failures.append(f"missing section: {heading}")
    if not re.search(r"(?im)^\*\*Door:\*\*\s*(?:one-way|two-way)\s*$", answer):
        failures.append("Door must be one-way or two-way")
    sections = re.split(r"(?m)^## (.+?)\s*$", answer)
    findings_section = next((sections[i + 1] for i in range(1, len(sections) - 1, 2)
                             if sections[i] == "Remaining review findings"), "")
    for finding in fixture["findings"]:
        if finding not in findings_section:
            failures.append(f"missing finding under Remaining review findings: {finding}")
    for value in fixture["commands_and_paths"]:
        if value not in answer:
            failures.append(f"missing command or path: {value}")
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("skills_dir", nargs="?", type=Path,
                        default=Path(os.environ.get("CLAUDE_CONFIG_DIR", Path.home() / ".claude")) / "skills",
                        help="directory containing factory-pr/ (default: installed Claude skills)")
    args = parser.parse_args()
    skills = args.skills_dir.expanduser().resolve()
    if not (skills / "factory-pr" / "SKILL.md").is_file():
        parser.error(f"missing factory-pr/SKILL.md in {skills}")
    fixture = json.loads((HERE / "fixture.json").read_text())
    prompt = (f"Read {skills / 'pr' / 'SKILL.md'} and follow it to draft a pull request description. "
              "Return only the finished description. Preserve the findings verbatim and every command and path.\n\n"
              + fixture["input"])
    try:
        with tempfile.TemporaryDirectory(prefix="pr-eval-") as work:
            result = subprocess.run(
                ["claude", "-p", prompt, "--add-dir", str(skills), "--permission-mode", "dontAsk",
                 "--tools", "Read", "--allowedTools", "Read", "--output-format", "json"],
                cwd=work, capture_output=True, text=True, timeout=600, check=False)
        if result.returncode:
            print(f"claude failed ({result.returncode}): {result.stderr or result.stdout}", file=sys.stderr)
            return 1
        response = json.loads(result.stdout)
        answer = response["result"]
    except (OSError, subprocess.TimeoutExpired, ValueError, KeyError) as exc:
        print(f"eval failed: {exc}", file=sys.stderr)
        return 1
    print(answer)
    failures = check(answer, fixture)
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    if failures:
        return 3
    print("\nAll assertions passed.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
