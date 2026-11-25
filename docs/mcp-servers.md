# MCP servers for GitHub and Azure DevOps

An MCP server lets your coding agent read and change things on your Git host, such as pull requests, work items and builds. Jidoka does not need one: `jidoka pr open`, `sync` and `wait` use the `gh` and `az` commands. Add one only when you want the agent to look things up itself while it works.

The repository's `.mcp.json` lists project-wide servers and has no credentials, so keep it that way. Add the servers below for yourself instead, in your user settings, so no token or organization name reaches a commit. Do the same for IDE servers such as RustRover or Rider: their port changes with every IDE session, so a committed entry fails with `ECONNREFUSED` whenever the IDE is closed.

## Side by side

| | GitHub | Azure DevOps |
|---|---|---|
| Server | [GitHub MCP server](https://github.com/github/github-mcp-server), remote at `https://api.githubcopilot.com/mcp/` | [`@azure-devops/mcp`](https://github.com/microsoft/azure-devops-mcp), run locally with `npx` |
| Needs | A GitHub personal access token | Node.js 20 or later and your organization name |
| Auth with no secret in a file | Keep the token in the `GITHUB_PAT` environment variable | `--authentication azcli` uses your `az login` session |
| Auth with a token | `Authorization: Bearer` header | `--authentication pat`, token in `PERSONAL_ACCESS_TOKEN` |

Microsoft also hosts a remote Azure DevOps server, but its sign-in needs dynamic OAuth client registration, which Microsoft lists as unsupported for Claude Code and Codex. Use the local server for them, as below. See [Microsoft's overview](https://learn.microsoft.com/en-us/azure/devops/mcp-server/mcp-server-overview) for the current state.

## Claude Code

Add both to your user settings. Replace `<org>` with your Azure DevOps organization name, the part after `dev.azure.com/`.

```bash
az login
claude mcp add --scope user azure-devops -- npx -y @azure-devops/mcp <org> --authentication azcli
```

```bash
export GITHUB_PAT=<token>
claude mcp add-json --scope user github '{"type":"http","url":"https://api.githubcopilot.com/mcp/","headers":{"Authorization":"Bearer ${GITHUB_PAT}"}}'
```

Claude Code expands `${GITHUB_PAT}` when it starts, so the token stays in your environment and not in the settings file. Start Claude Code from a shell that has `GITHUB_PAT` set. GitHub's guide notes that `claude mcp add-json` can reject an HTTP server on Windows; use `claude mcp add --transport http github https://api.githubcopilot.com/mcp/ --header "Authorization: Bearer ${GITHUB_PAT}"` there.

## Azure DevOps with a token

If you cannot use `az login`, choose a token method. The server reads the token from the environment, so never put it in a configuration file:

```bash
export PERSONAL_ACCESS_TOKEN="$(printf '%s' '<email>:<pat>' | base64)"
claude mcp add --scope user azure-devops -- npx -y @azure-devops/mcp <org> --authentication pat
```

The value must be the base64 encoding of `<email>:<pat>`; any non-empty email works. `--authentication envvar` instead reads a bearer token from `ADO_MCP_AUTH_TOKEN`. In PowerShell, set the variable with `$env:PERSONAL_ACCESS_TOKEN = ...` before you start the agent.

## Codex and GitHub Copilot

Codex uses the same server and arguments:

```bash
codex mcp add azure-devops -- npx -y @azure-devops/mcp <org> --authentication azcli
```

For GitHub Copilot, follow the [Azure DevOps MCP README](https://github.com/microsoft/azure-devops-mcp) and the [GitHub MCP server guides](https://github.com/github/github-mcp-server/tree/main/docs/installation-guides), which cover the current setup for each client. The server, organization name and sign-in choices above are the same.
