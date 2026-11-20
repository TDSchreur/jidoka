---
name: dashboard-check
description: Run the dashboard browser tests and review dashboard changes against the accessibility guide. Use after changing dashboard HTML, CSS or JavaScript, or its tests.
disable-model-invocation: true
---

# Dashboard check

1. Read `.github/instructions/accessibility.instructions.md` and compare the changed dashboard files (`dashboard/`) against it. Report each deviation with file and line.
2. Run every browser test and report each failure with its output:

   ```sh
   for t in tests/dashboard-*.py; do python3 "$t" || echo "FAILED: $t"; done
   ```

   They need `python3` and Chromium or Google Chrome (set `CHROMIUM` to choose one). Missing tools are a blocker to install. On WSL2 or with `JIDOKA_SKIP_BROWSER_TESTS=1` the tests are skipped by `scripts/verify.sh`; say so and do not claim they passed.
3. Finish with `scripts/verify.sh --quick`.
