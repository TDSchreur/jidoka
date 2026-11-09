"""Unit tests for the retry decision in tests/dashboard_browser.py; they need no browser."""
import contextlib
import io
import os
import subprocess
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dashboard_browser as harness


def attempts(*outcomes):
    """An attempt callable that returns or raises each outcome in turn and counts its calls."""
    queue = list(outcomes)
    calls = []

    def attempt():
        calls.append(1)
        outcome = queue.pop(0)
        if isinstance(outcome, Exception):
            raise outcome
        return outcome
    attempt.calls = calls
    return attempt


def run(attempt):
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        try:
            result = harness.run_with_retry(attempt)
        except Exception as error:
            return error, err.getvalue(), len(attempt.calls)
    return result, err.getvalue(), len(attempt.calls)


class RetryTests(unittest.TestCase):
    def test_success_runs_once_without_retry(self):
        result, err, count = run(attempts({"ok": True}))
        self.assertEqual(result, {"ok": True})
        self.assertEqual(count, 1)
        self.assertEqual(err, "")

    def test_failing_result_is_returned_without_retry(self):
        result, err, count = run(attempts({"ok": False, "message": "assertion"}))
        self.assertEqual(result, {"ok": False, "message": "assertion"})
        self.assertEqual(count, 1)
        self.assertEqual(err, "")

    def test_crash_then_success_retries_once_and_logs(self):
        result, err, count = run(attempts(harness.BrowserFailure("exited", "exit code 5"), {"ok": True}))
        self.assertEqual(result, {"ok": True})
        self.assertEqual(count, 2)
        self.assertIn("exited", err)
        self.assertIn("retrying", err)

    def test_timeout_then_success_logs_the_reason(self):
        result, err, count = run(attempts(harness.BrowserFailure("no result in time", "waited 120s"), {"ok": True}))
        self.assertEqual(result, {"ok": True})
        self.assertEqual(count, 2)
        self.assertIn("no result in time", err)
        self.assertIn("retrying", err)

    def test_two_crashes_fail_after_one_retry(self):
        error, _, count = run(attempts(harness.BrowserFailure("exited", "a"), harness.BrowserFailure("exited", "b")))
        self.assertIsInstance(error, harness.BrowserFailure)
        self.assertEqual(count, 2)

    def test_other_errors_are_not_retried(self):
        error, _, count = run(attempts(ValueError("bug")))
        self.assertIsInstance(error, ValueError)
        self.assertEqual(count, 1)


class StubBrowserTests(unittest.TestCase):
    """A stub browser process that exits at once is told apart from a posted result."""

    def test_exiting_browser_is_a_browser_failure(self):
        import queue
        stub = [sys.executable, "-c", "import sys; sys.exit(7)"]
        with self.assertRaises(harness.BrowserFailure) as caught:
            harness.wait_for_result(None, queue.Queue(), (100, 100), None, (), command=stub)
        self.assertIn("exit code 7", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
