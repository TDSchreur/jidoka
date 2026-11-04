# Skills

`factory2` is the local workflow skill. The other skill directories are unmodified upstream copies. Each skill lives in its own directory with a `SKILL.md` and any supporting files. The CLI does not install skills; agents can read the files from this checkout or install the directories using their normal installation process.

- [mattpocock/skills](https://github.com/mattpocock/skills), commit [`59db609`](https://github.com/mattpocock/skills/tree/59db609eb1e18aaf59b63284d0e52255aff0c63c): `grill-with-docs`, `to-spec`, `to-tickets`, `implement-spec`, `tdd`, `diagnosing-bugs`, `code-review`, `pr`, `setup-matt-pocock-skills`, `domain-modeling`, `codebase-design`, and `grilling`.
- [github/awesome-copilot](https://github.com/github/awesome-copilot), commit [`0fe6545`](https://github.com/github/awesome-copilot/tree/0fe6545b950671641b2b14b44eb56678fea6c6b4): `security-review` (called `security-scan` in the original Factory).
- [joshuadavidthomas/agent-skills](https://github.com/joshuadavidthomas/agent-skills), commit [`516dee7`](https://github.com/joshuadavidthomas/agent-skills/tree/516dee7a422b90937b2958d11c03694154ab9c09): `rust`.

The upstream projects use the MIT license; their license texts are in [`third-party/`](../third-party/). These instructions may invoke other skills or expect project-specific setup, so review them before use. Updates are manual: compare with upstream and copy the desired changes along with their supporting files.
