# Skills

The local workflow skills, one per workflow mode, live in the project skill discovery directory alongside `triage` and `rust-best-practices`:

- [`factory`](../.agents/skills/factory/SKILL.md) grills, writes a spec and tickets, builds test-first with `tdd`, then runs code and security review.
- [`factory-balanced`](../.agents/skills/factory-balanced/SKILL.md) plans lightly with explicit acceptance criteria, builds with [`implement-spec-balanced`](implement-spec-balanced/SKILL.md) (implementation first, then behavior tests), and runs one code review, plus a security review for security-sensitive diffs.
- [`factory-fast`](../.agents/skills/factory-fast/SKILL.md) plans lightly, builds without TDD using [`implement-spec-fast`](implement-spec-fast/SKILL.md), and skips the Check stage.

`implement-spec-balanced` and `implement-spec-fast` are local implementation skills; the remaining skill directories come from the upstream projects listed below. Each skill lives in its own directory with a `SKILL.md` and any supporting files. `factory install --agent codex` installs the three workflows, `triage`, `rust-best-practices`, `github-actions-hardening`, and their required skills, including `tdd`, for Codex; `claude` and `copilot` are also supported, and a comma-separated list such as `codex,claude` installs for several agents. Each install replaces the bundled skill directories. Other skills in this directory can be installed using the agent's normal installation process.

- [mattpocock/skills](https://github.com/mattpocock/skills), commit [`59db609`](https://github.com/mattpocock/skills/tree/59db609eb1e18aaf59b63284d0e52255aff0c63c): `grill-with-docs`, `to-spec`, `to-tickets`, `implement-spec`, `tdd`, `diagnosing-bugs`, `code-review`, `pr`, `setup-matt-pocock-skills`, `domain-modeling`, `codebase-design`, `grilling`, and `triage`.
- [github/awesome-copilot](https://github.com/github/awesome-copilot), commit [`0fe6545`](https://github.com/github/awesome-copilot/tree/0fe6545b950671641b2b14b44eb56678fea6c6b4): `security-review` (called `security-scan` in the original Factory); `github-actions-hardening` comes from commit [`9654314`](https://github.com/github/awesome-copilot/tree/96543145e9cb97711020b57b9d88f968062c4fc7).
- [joshuadavidthomas/agent-skills](https://github.com/joshuadavidthomas/agent-skills), commit [`516dee7`](https://github.com/joshuadavidthomas/agent-skills/tree/516dee7a422b90937b2958d11c03694154ab9c09): `rust`.
- [morganlinton/BeautifulRust](https://github.com/morganlinton/BeautifulRust), commit [`edd3e86`](https://github.com/morganlinton/BeautifulRust/tree/edd3e86087eda07983251056508fe3b8856d85d7): `rust-best-practices`.

The upstream projects use the MIT license; their license texts are in [`third-party/`](../third-party/). These instructions may invoke other skills or expect project-specific setup, so review them before use. Updates are manual: compare with upstream and copy the desired changes along with their supporting files.
