# Jidoka

**The AI software factory.** Jidoka takes a piece of work from idea to a reviewed pull request, using coding agents you already run: Claude Code, Codex or GitHub Copilot.

[![CI](https://github.com/TDSchreur/jidoka/actions/workflows/ci.yml/badge.svg?branch=develop)](https://github.com/TDSchreur/jidoka/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/TDSchreur/jidoka)](https://github.com/TDSchreur/jidoka/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Every run follows the same line: **Define → Build → Check → Finish**. Jidoka gives your agent the workflow as skills, gives each run its own git worktree, and shows all runs on a local dashboard, with what needs you at the top. Each run ends with a pull request and green CI.

## Why "Jidoka"?

*Jidoka* (自働化) is one of the two pillars of the Toyota Production System, often translated as **automation with a human touch**. A Jidoka machine works on its own, but it stops itself the moment something is wrong, so that a person can fix the cause before bad work moves down the line. Quality is built in at each step instead of inspected at the end.

That is the idea behind this project:

- **Automation.** Agents write the spec, the code, the tests and the pull request.
- **Stop the line.** Reviews in Check, the project's own verification and CI stop a run when something is wrong, and send it back to Build for a Rework.
- **A human touch.** When a run needs a decision, it stops and asks. The dashboard's andon strip, named after the signal board above a factory line, lists every run that needs you.

## Install

You need Linux x86-64 or Windows x86-64, `git`, and at least one supported coding agent. Opening pull requests also needs the GitHub CLI (`gh`), or the Azure CLI (`az`) with the `azure-devops` extension for Azure Repos.

**Linux**

```sh
curl -fsSL https://github.com/TDSchreur/jidoka/releases/latest/download/install.sh | sh
```

**Windows** (PowerShell)

```powershell
irm https://github.com/TDSchreur/jidoka/releases/latest/download/install.ps1 | iex
```

The installer downloads the `jidoka` command, checks it against the release checksum, and runs `jidoka install`, which asks which agents to install the skills for. Re-run the same one-liner later to update; the skills refresh automatically after an upgrade.

Windows Defender sometimes flags the unsigned Windows build; see [what to do](docs/installation.md#windows-defender-blocks-the-download). For unattended installs, several Claude Code configs and building from source, see the [installation guide](docs/installation.md).

## Usage

1. **Start the dashboard** and keep it running in its own terminal:

   ```sh
   jidoka dashboard
   ```

   The command prints the dashboard URL (<http://127.0.0.1:4311>, or another port when 4311 is taken); open it to follow your runs.

2. **Prepare a repository** once: in your project, ask your agent to run `/jidoka-setup`. It records the issue tracker and conventions the workflows use.

3. **Start a run.** Ask your agent to run `/jidoka` with your task, for example:

   ```text
   /jidoka Add a --json option to the export command
   ```

   `/jidoka` picks a workflow mode for the task. You can also pick one yourself:

   | Mode | Use it for |
   | --- | --- |
   | `/jidoka-full` | Risky or unclear work: an interview, a spec and tickets, project-directed tests and code review. |
   | `/jidoka-balanced` | Everyday features and fixes: a short plan, tests after the code, one code review. |
   | `/jidoka-fast` | Small, clear changes: no review stage, but still verified and green in CI. |

4. **Review the pull request** the run opens. The agent watches CI and fixes failures until it is green.

5. **Report problems** (optional): ask your agent to run `/jidoka-run-report` with a run id. It looks for Jidoka bugs and avoidable token usage in that run and, after you approve the text, files a detailed issue in the Jidoka repository.

The installed skills also work on their own, outside a run; see [Skills you can use directly](#skills-you-can-use-directly).

Useful commands:

| Command | What it does |
| --- | --- |
| `jidoka doctor` | Checks that the repository and your machine are ready. |
| `jidoka run inspect <id>` | Shows a run's stage, workspace, tickets and pull request. |
| `jidoka run stats <id>` | Shows a run's token usage and token-based estimated cost at API list prices (not subscription charges). |
| `jidoka run cleanup <id>` | Removes a run's worktree and branch after its pull request merged. |

Run `jidoka --help` for every command.

### Skills you can use directly

`jidoka install` adds these skills for every agent you chose. Call one by name, for example `/jidoka-security-review src/api`, or describe the task and let the agent pick it.

| Skill | What it does |
| --- | --- |
| [`/jidoka-setup`](skills/jidoka-setup/SKILL.md) | Prepares a repository once: issue tracker, triage labels, domain docs and verification commands. |
| [`/jidoka-setup-knowledge-repo`](skills/jidoka-setup-knowledge-repo/SKILL.md) | Creates or fills a knowledge repo: your repositories, hosts, checkouts and shared platform notes. |
| [`/jidoka-cross-repo-change`](skills/jidoka-cross-repo-change/SKILL.md) | Guides one change across several repositories: shared spec, one run per repository, a change group. |
| [`/jidoka-awesome-copilot`](skills/jidoka-awesome-copilot/SKILL.md) | Adds skills and instructions from [github/awesome-copilot](https://github.com/github/awesome-copilot) to a project, adapted to its layout, for Codex, Claude and Copilot. Also updates or removes them. |
| [`/jidoka-run-report <run-id>`](skills/jidoka-run-report/SKILL.md) | Checks a run for Jidoka bugs and token waste and files an issue in the Jidoka repository after you approve it. |
| [`/jidoka-code-review <ref>`](skills/jidoka-code-review/SKILL.md) | Reviews changes since a commit or branch against the project's standards and the originating spec. |
| [`/jidoka-security-review`](skills/jidoka-security-review/SKILL.md) | Scans a path, commit range or the whole project for vulnerabilities. |
| [`/jidoka-grilling`](skills/jidoka-grilling/SKILL.md) | Stress-tests a plan or decision with pointed questions. |
| [`/jidoka-grill-with-docs`](skills/jidoka-grill-with-docs/SKILL.md) | Same interview, and records glossary terms and ADRs as decisions land. |
| [`/jidoka-domain-modeling`](skills/jidoka-domain-modeling/SKILL.md) | Sharpens the domain language: `GLOSSARY.md` and ADRs. |
| [`/jidoka-improve-architecture`](skills/jidoka-improve-architecture/SKILL.md) | Finds refactoring opportunities and files the one you pick as a spec. |
| [`/jidoka-to-spec`](skills/jidoka-to-spec/SKILL.md) | Turns the current conversation into a spec in the issue tracker. |
| [`/jidoka-to-tickets`](skills/jidoka-to-tickets/SKILL.md) | Breaks a spec or plan into tickets in the issue tracker. |
| [`/jidoka-triage`](.agents/skills/jidoka-triage/SKILL.md) | Triages issues and external pull requests and writes agent-ready briefs. |
| [`/jidoka-tdd`](skills/jidoka-tdd/SKILL.md) | Builds a feature or fixes a bug test-first. |
| [`/jidoka-pr`](skills/jidoka-pr/SKILL.md) | Writes a pull request description that reviewers can scan quickly. |

### Where runs are stored

Jidoka keeps its data in one folder per user:

- Linux and macOS: `$XDG_DATA_HOME/jidoka/`, or `~/.local/share/jidoka/` when `XDG_DATA_HOME` is not set.
- Windows: `%LOCALAPPDATA%\jidoka\`.

| File | What it holds |
| --- | --- |
| `runs.json` | Every run: stage, status, gates, Findings, timeline, reports, summaries and token usage. |
| `groups.json` | Change groups: title, spec and member run ids. Owned by the dashboard like `runs.json`; change it with `jidoka group`. |
| `plans/<run-id>.md` | A run's saved plan, kept after its worktree is cleaned up. |
| `session-token` | The token the CLI sends to the dashboard, replaced on each dashboard start and readable only by you. |
| `dashboard.json` | The port, pid and build of the running dashboard, written when it starts and removed when it stops. The CLI reads it to find the dashboard. |

There is one data folder and one dashboard per user. The dashboard owns `runs.json` and rewrites it on every update, so read runs with `jidoka run inspect <id>` (`--json` for the raw record) and change them with `jidoka run update`, not by editing the file.

## Documentation

| Guide | Covers |
| --- | --- |
| [Installation](docs/installation.md) | Install options, updating, skills folders, building from source, Windows Defender |
| [Workflows and skills](docs/workflows.md) | The three modes stage by stage, Reworks, the skills, agent differences |
| [Runs and the dashboard](docs/runs.md) | The dashboard, `jidoka run` commands, run context, continuing a run |
| [Changes across repositories](docs/multi-repo.md) | Change groups: one spec, several runs and pull requests, prerequisites and milestones |
| [Getting started with several repositories](docs/multi-repo-getting-started.md) | The knowledge repo and the two skills that guide a first cross-repo change |
| [Pull requests and CI](docs/pull-requests.md) | `jidoka pr open`, `sync` and `wait` on GitHub and Azure Repos |
| [Token usage](docs/usage-tracking.md) | Token counts, estimated cost, pauses, request analysis |
| [Model routing](docs/model-routing.md) | Choosing the model and effort per Stage |
| [Checking readiness](docs/doctor.md) | What `jidoka doctor` checks |
| [Glossary](GLOSSARY.md) | The terms Jidoka uses |

## Network and privacy

This program will not transfer any information to other networked systems unless specifically requested by the user.

- The release installers download from GitHub Releases too.
- `jidoka run begin` fetches the chosen base from the repository's configured Git remote. `jidoka pr open` and `jidoka pr sync` fetch or push to that remote as needed.
- `jidoka pr open`, `jidoka pr sync`, `jidoka pr wait`, `jidoka run sync`, `jidoka run cleanup`, and the dashboard use `gh` or `az` to access the user's pull requests and CI on GitHub or Azure DevOps. The dashboard may refresh recorded pull request status while running. `jidoka doctor` checks the selected provider's CLI and authentication status.

These services have their own privacy policies: [GitHub](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement) and [Microsoft (Azure DevOps)](https://privacy.microsoft.com/en-us/privacystatement). A configured Git remote may use another host with its own policy. Coding agents and tools invoked by them operate separately under their own policies and user configuration.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the branch model and how to verify a change, and [SECURITY.md](SECURITY.md) to report a vulnerability.

## Acknowledgements

Many of the bundled skills started in [mattpocock/skills](https://github.com/mattpocock/skills) and [github/awesome-copilot](https://github.com/github/awesome-copilot); [skills/README.md](skills/README.md) lists their origins. The dashboard uses [Preact](https://preactjs.com) and [htm](https://github.com/developit/htm). Third-party licenses are in [`third-party/`](third-party/). Jidoka grew out of the earlier [tds-factory project](https://github.com/TDSchreur/tds-factory).

## License

[MIT](LICENSE) © T.D. Schreur
