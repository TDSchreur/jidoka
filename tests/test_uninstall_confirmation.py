"""Interactive uninstall confirmation preserves unrelated data."""
import os
from pathlib import Path
import select
import subprocess
import tempfile
import unittest

from isolated_env import isolated_env


@unittest.skipUnless(os.name == "posix", "requires a pseudoterminal")
class UninstallConfirmation(unittest.TestCase):
    def test_confirm_and_decline(self):
        binary = Path(__file__).resolve().parent.parent / "target/debug/jidoka"
        for answer, retained in (("yes\n", False), ("no\n", True)):
            with self.subTest(answer=answer), tempfile.TemporaryDirectory() as home:
                data = Path(home) / "data/jidoka"
                data.mkdir(parents=True)
                (data / "runs.json").write_text("runs", encoding="utf-8")
                (data / "other").write_text("mine", encoding="utf-8")
                master, slave = os.openpty()
                env = isolated_env(home, CODEX_HOME=str(Path(home) / "codex"))
                process = subprocess.Popen([str(binary), "uninstall", "--agent", "codex"],
                                           stdin=slave, stdout=slave, stderr=slave, env=env)
                os.close(slave)
                try:
                    ready, _, _ = select.select([master], [], [], 10)
                    self.assertTrue(ready, "confirmation prompt did not appear")
                    self.assertIn(b"Remove Jidoka run data and the model routing policy?", os.read(master, 4096))
                    os.write(master, answer.encode())
                    self.assertEqual(process.wait(timeout=10), 0)
                    self.assertEqual((data / "runs.json").exists(), retained)
                    self.assertTrue((data / "other").exists())
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
                    os.close(master)
