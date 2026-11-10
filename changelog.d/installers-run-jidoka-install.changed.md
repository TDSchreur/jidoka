## Changed

- `install.sh` and `install.ps1` no longer ask which agents to install skills for themselves; after placing the binary they run `jidoka install`, which asks (reading from the terminal when the script is piped) and honors `JIDOKA_AGENT` and `JIDOKA_CLAUDE_CONFIG_DIRS`.
