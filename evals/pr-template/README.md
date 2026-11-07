# PR template eval

This eval checks a PR description against a fixed diff, before-and-after test result, and two remaining review findings. It uses a paid Claude call and is not part of CI. Run it after installing the skills to measure the installed `pr` skill:

```sh
python3 evals/pr-template/run.py
python3 evals/pr-template/run.py /path/to/snapshot/skills
```

The optional directory must contain `pr/SKILL.md` and `humanizer/SKILL.md`. The default is `$CLAUDE_CONFIG_DIR/skills`, or `~/.claude/skills`. The runner gives Claude read-only tools in a temporary directory and asks it to read those exact files. It prints the description and checks that the required sections, a valid Door, both findings under their heading, and the fixture's commands and paths are present. It also checks that the description starts with `## Summary`. Exit status 0 means all assertions passed, 3 means an assertion failed, 1 means the call failed, and 2 means the arguments or skill directory are invalid. Output varies between calls; compare repeated runs when evaluating a change.

Run the offline runner tests without a model call (they skip on Windows because the fake executable uses a Unix shebang):

```sh
python3 -m unittest discover -s evals/pr-template -p 'test_*.py'
```
