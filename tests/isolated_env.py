"""An environment for running the jidoka binary without touching the real home of whoever runs the tests."""
import os
from pathlib import Path

HOME_VARIABLES = ("HOME", "USERPROFILE", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "APPDATA", "LOCALAPPDATA",
                  "CLAUDE_CONFIG_DIR", "CODEX_HOME", "COPILOT_HOME")


def isolated_env(home, **overrides):
    """The current environment with every home, config and data location pointing inside `home`."""
    home = Path(home)
    env = dict(os.environ)
    env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / "config"),
               APPDATA=str(home / "config"), XDG_DATA_HOME=str(home / "data"), LOCALAPPDATA=str(home / "data"),
               CLAUDE_CONFIG_DIR=str(home / ".claude"), CODEX_HOME=str(home / ".codex"),
               COPILOT_HOME=str(home / ".copilot"))
    env.pop("JIDOKA_AGENT", None)
    env.pop("JIDOKA_CLAUDE_CONFIG_DIRS", None)
    env.update(overrides)
    return env
