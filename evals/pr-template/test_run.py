"""Exercise the evaluation CLI with a local stand-in for Claude."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
GOOD = """## Summary

src/parser.py and tests/test_parser.py

## Evidence

`python -m unittest tests.test_parser` failed before and passed after.

## Merge Danger

**Door:** two-way

## Remaining review findings

- src/parser.py:12: Empty input still raises an exception.
- tests/test_parser.py:24: The Unicode whitespace case is missing.
"""


@unittest.skipIf(os.name == "nt", "the fake Claude executable uses a Unix shebang")
class EvalTest(unittest.TestCase):
    def run_eval(self, answer, status=0):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "skills" / "pr").mkdir(parents=True)
            (root / "skills" / "pr" / "SKILL.md").write_text("fixture")
            executable = root / "claude"
            executable.write_text("#!" + sys.executable + "\nimport json, sys\nprint(json.dumps({'result': " + repr(answer) + "}))\nsys.exit(" + str(status) + ")\n")
            executable.chmod(0o755)
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ.get("PATH", ""))
            return subprocess.run([sys.executable, str(HERE / "run.py"), str(root / "skills")],
                                  capture_output=True, text=True, env=env, check=False)

    def test_accepts_complete_description(self):
        result = self.run_eval(GOOD)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_missing_findings_and_preamble(self):
        result = self.run_eval("Here it is\n" + GOOD.replace("- tests/test_parser.py:24: The Unicode whitespace case is missing.", ""))
        self.assertEqual(result.returncode, 3)
        self.assertIn("answer must start", result.stderr)
        self.assertIn("missing finding", result.stderr)

    def test_rejects_invalid_door_and_missing_command(self):
        result = self.run_eval(GOOD.replace("two-way", "unknown").replace("python -m unittest tests.test_parser", "test command"))
        self.assertEqual(result.returncode, 3)
        self.assertIn("Door must", result.stderr)
        self.assertIn("missing command or path", result.stderr)

    def test_reports_claude_failure(self):
        result = self.run_eval("usage limit", 1)
        self.assertEqual(result.returncode, 1)
        self.assertIn("claude failed", result.stderr)


if __name__ == "__main__":
    unittest.main()
