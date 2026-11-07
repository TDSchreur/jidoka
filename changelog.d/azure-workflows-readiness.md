## Added

- `factory doctor` has a Provider section. It shows whether origin is on GitHub or Azure Repos, whether `gh`, or `az` with the `azure-devops` extension, is installed, and whether it is signed in (`gh auth status`, `az account show`, or `AZURE_DEVOPS_EXT_PAT` being set). It never prints the token and says what it did not check: token validity, project permissions and branch policies. These are warnings, so a project that was ready stays ready.
- Request analysis puts `az repos pr create` and `update` under PR creation and push, and `az pipelines runs`, `az repos pr policy list` and `az devops invoke` on pull request statuses under CI watch.

## Changed

- The workflows' Pull request and CI procedure names the tool for each host: `gh` for GitHub, `az` with the `azure-devops` extension for Azure Repos. It gives the manual commands for both, uses `factory pr sync` for the base state on Azure Repos, and on any other host has the agent open the pull request with the host's own tooling and record it with `--pr`.
