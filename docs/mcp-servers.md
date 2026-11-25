# MCP servers for GitHub and Azure DevOps

An MCP server lets your coding agent read and change things on your Git host, such as pull requests, work items and builds. Jidoka does not need one: `jidoka pr open`, `sync` and `wait` use the `gh` and `az` commands. Add one only when you want the agent to look things up itself while it works.

The repository's `.mcp.json` lists project-wide servers and has no credentials, so keep it that way. Add the servers below for yourself instead, in your user settings, so no token or organization name reaches a commit. Do the same for IDE servers such as RustRover or Rider: their port changes with every IDE session, so a committed entry fails with `ECONNREFUSED` whenever the IDE is closed.

## Side by side

| | GitHub | Azure DevOps |
|---|---|---|
| Server | [GitHub MCP server](https://github.com/github/github-mcp-server), remote at `https://api.githubcopilot.com/mcp` | [`@azure-devops/mcp`](https://github.com/microsoft/azure-devops-mcp), run locally with `npx` |
| Needs | A GitHub personal access token | Node.js 20 or later and your organization name |
| Install command | `claude mcp add-json --scope user github ...` | `claude mcp add --scope user azure-devops -- npx -y @azure-devops/mcp <org> ...` |
| Auth without a stored token | Not available: the remote server needs a token | `--authentication azcli` uses your `az login` session |
| Auth with a token | `Authorization: Bearer` header, stored in your user settings | `--authentication pat`, token in `PERSONAL_ACCESS_TOKEN` |

Microsoft also hosts a remote Azure DevOps server, but its sign-in needs dynamic OAuth client registration, which Microsoft lists as unsupported for Claude Code and Codex. Use the local server for them, as below. See [Microsoft's overview](https://learn.microsoft.com/en-us/azure/devops/mcp-server/mcp-server-overview) for the current state.

## Claude Code

Add both to your user settings. Replace `<org>` with your Azure DevOps organization name, the part after `dev.azure.com/`.

```bash
az login
claude mcp add --scope user azure-devops -- npx -y @azure-devops/mcp <org> --authentication azcli
```

```bash
claude mcp add-json --scope user github '{"type":"http","url":"https://api.githubcopilot.com/mcp","headers":{"Authorization":"Bearer <token>"}}'
```

Claude Code writes the GitHub token into your user settings file, outside the repository, so it never reaches a commit. Claude Code documents `${VAR}` expansion for `.mcp.json` only, so this guide does not rely on it for user settings. Never paste the token into the repository's `.mcp.json`. The Azure DevOps entry above stores no token.

GitHub's guide notes that `claude mcp add-json` can reject an HTTP server on Windows. Use this form there:

```powershell
claude mcp add --scope user --transport http github https://api.githubcopilot.com/mcp -H "Authorization: Bearer <token>"
```

## Azure DevOps with a token

If you cannot use `az login`, choose a token method. The server reads the token from the environment, so never put it in a configuration file:

```bash
export PERSONAL_ACCESS_TOKEN="$(printf '%s' '<email>:<pat>' | base64)"
claude mcp add --scope user azure-devops -- npx -y @azure-devops/mcp <org> --authentication pat
```

The value must be the base64 encoding of `<email>:<pat>`; the server's guide says the email can be any non-empty value. `--authentication envvar` instead reads a bearer token from `ADO_MCP_AUTH_TOKEN`. In PowerShell, set the variable for the current session before you start the agent:

```powershell
$env:PERSONAL_ACCESS_TOKEN = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('<email>:<pat>'))
```

## Codex and GitHub Copilot

Codex uses the same server and arguments:

```bash
codex mcp add azure-devops -- npx -y @azure-devops/mcp <org> --authentication azcli
```

For GitHub Copilot, follow the [Azure DevOps MCP README](https://github.com/microsoft/azure-devops-mcp) and the [GitHub MCP server guides](https://github.com/github/github-mcp-server/tree/main/docs/installation-guides), which cover the current setup for each client. The server, organization name and sign-in choices above are the same.
