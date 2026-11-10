# Installation

How to install, update and configure Jidoka. The [README](../README.md#install) has the short version.

See the [Code signing policy](../README.md#code-signing-policy).

## Requirements

You need Linux x86-64 or Windows x86-64, `git`, and a supported coding agent (Codex, Claude Code, or GitHub Copilot). To open pull requests and watch CI, the workflows need the `gh` CLI on GitHub, or the Azure CLI (`az`) with its `azure-devops` extension on Azure Repos. Building from source also needs a Rust toolchain with Cargo.

## Install a release

Install the latest release with one command:

Linux x86-64:

```sh
curl -fsSL https://github.com/TDSchreur/jidoka/releases/latest/download/install.sh | sh
```

Windows x86-64 (PowerShell):

```powershell
irm https://github.com/TDSchreur/jidoka/releases/latest/download/install.ps1 | iex
```

Both installers download the prebuilt CLI, verify its release checksum, ask which agents to install the Jidoka skills for (Codex, Claude Code, GitHub Copilot, one or more), and install them. Answer with numbers or names, such as `1,3` or `claude copilot`, or `all`. To skip the prompt, set `JIDOKA_AGENT` to a comma-separated list before running the installer, for example `JIDOKA_AGENT=codex,claude` (`$env:JIDOKA_AGENT = 'codex,claude'` in PowerShell). Without `JIDOKA_AGENT` and without a terminal to ask on, the installer stops before installing anything. Each install replaces the bundled skill directories with the versions shipped in the CLI, so local edits to those skills are lost; other skills in the same folder are left alone. To install skills for other agents later, run `jidoka install --agent claude` (or `codex` / `copilot`, or a comma-separated list such as `codex,claude`). Re-running the one-line installer updates both the CLI and its skills.

## Check for updates and update

To see whether a newer release exists, run `jidoka check`. It looks up the latest release, says whether you are up to date, and exits 0 either way (1 on an error, such as no network). A development build cannot be compared, so it only names the latest release.

To update, run `jidoka update`. It downloads the latest release for your platform, verifies its checksum, replaces the installed `jidoka`, and reinstalls the skills for the agents that already have them. Use `jidoka update --agent codex,claude` to choose the agents yourself. If you run a development build, it updates to the latest release. It supports Linux and Windows on x86-64 only. Restart a running dashboard afterwards. Re-running the one-line installer does the same.

Stop the dashboard before upgrading when you can; `jidoka update` renames a running Windows executable aside, but the dashboard keeps running the old version until you restart it. The release workflow checks Windows runtime imports and starts the Windows executable on a Windows runner.

## Skills folders and several Claude Code configs

Claude Code skills go to `$CLAUDE_CONFIG_DIR/skills` when `CLAUDE_CONFIG_DIR` is set (for example `~/.claude-work`), otherwise `~/.claude/skills`. Codex skills likewise honor `CODEX_HOME`, and Copilot skills honor `COPILOT_HOME` (otherwise `~/.copilot/skills`).

To serve more than one Claude Code config folder, such as a personal `~/.claude` and a work `~/.claude-work`, repeat `--config-dir <path>` on `jidoka install`, `jidoka update` and `jidoka doctor`:

```sh
jidoka install --agent claude --config-dir ~/.claude --config-dir ~/.claude-work
```

The flag applies to the `claude` agent only, and is an error when `--agent` leaves Claude out. Claude's folder is chosen in this order:

1. `--config-dir`, once or several times.
2. `CLAUDE_CONFIG_DIR`, when it is not empty.
3. `~/.claude`.

A leading `~` means your home folder. A relative path is rejected before anything is written, and a folder named twice is used once. `jidoka update` reinstalls the skills into each folder given, and `jidoka doctor` reports each folder by path.

The installers, `scripts/update-local.sh` and `scripts/switch-jidoka.sh` read an optional `JIDOKA_CLAUDE_CONFIG_DIRS` and pass each entry as `--config-dir`. Separate entries with `:` on Linux and `;` in PowerShell. Empty entries are skipped.

The dashboard and `jidoka run requests` read the same variable, with your system's path separator. They read Claude Code session logs from `projects` in the resolved folder and in every listed folder. Project-level `.claude/` folders are not affected.

## Uninstall

Stop the dashboard, then run `jidoka uninstall --agent codex,claude,copilot` with the agents whose bundled skills you installed. Use the same `--config-dir <path>` options as installation to select custom Claude configuration directories. Other skills and configuration are preserved.

The command removes the executable at the installer-managed location. On Windows it also removes the install directory from the user PATH and schedules the executable for removal after exit. Run data is retained unless you confirm its removal interactively; pass `--keep-data` to skip the prompt. Unattended invocations retain run data. Run workspaces and repositories are preserved.

If you installed with Cargo, also run `cargo uninstall jidoka`. To remove your configuration separately, delete the Jidoka configuration directory under `~/.config` (or `$XDG_CONFIG_HOME`) on Linux, or `%APPDATA%` on Windows.

## Build from source

To build from source instead, run:

```sh
cargo run -- --help
cargo run -- --version
cargo run -- dashboard
```

The Rocket server binds to localhost at <http://127.0.0.1:4311>; press Ctrl+C to stop it. It stores run progress under `$XDG_DATA_HOME/jidoka/` (or `~/.local/share/jidoka/` when unset) on Linux, and `%LOCALAPPDATA%\jidoka\` on Windows. Keep it running while using Jidoka. After a merge, `scripts/update-local.sh` updates the checkout and prompts for one or more agents; set `JIDOKA_AGENT=codex,claude` to select them without a prompt. To install from a checkout, use `cargo install --path . --locked`, ensure Cargo's bin directory is on PATH, and run `jidoka install --agent <codex|claude|copilot>`. Restart a previously running dashboard after upgrading.

## Which build is running

The dashboard footer and `jidoka --version` show which build is running: the version number, such as `0.3.1`, for an official release, and `dev <commit>`, such as `dev 1a2b3c4`, for any other build, including builds from a checkout. A build without git information, such as from a source archive, shows `dev (unknown commit)`. Official release builds set `JIDOKA_RELEASE=1`.

## Windows Defender blocks the download

The Windows release is not code-signed yet. Windows Defender can therefore flag `jidoka.exe` as a false positive (for example `Trojan:Win32/Bearfoos.A!ml`; the `!ml` suffix marks a machine-learning heuristic, not a known malware signature) and remove it. The installer then stops with a message that names the antivirus block, and nothing is installed.

1. Check the file. Download `jidoka-windows-x86_64.exe` and `jidoka-windows-x86_64.exe.sha256` from the [latest release](https://github.com/TDSchreur/jidoka/releases/latest), and compare the hash: `(Get-FileHash -Algorithm SHA256 .\jidoka-windows-x86_64.exe).Hash` must equal the value in the `.sha256` file. Do not continue when it differs.
2. Report the false positive to Microsoft at <https://www.microsoft.com/wdsi/filesubmission> as a software developer, with the detection name from Windows Security > Protection history.
3. Once the hash matches, restore the file from Windows Security > Protection history (Allow on device), or ask your administrator to allow it, and run the installer again. Prefer allowing this one file over excluding a whole folder such as `%TEMP%`.

Each new release has a new hash, so it can be flagged again until the executable is signed.

