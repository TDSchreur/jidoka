# Azure DevOps commands

Commands for [jidoka-pr-watch](../SKILL.md) on Azure Repos, using the Azure CLI with the `azure-devops` extension, signed in as the user (`az login` or `AZURE_DEVOPS_EXT_PAT`).

## Windows and quoting

On Windows the Azure CLI is `az.cmd`. PowerShell finds it when you type `az`, but arguments pass through `cmd.exe`, which mangles inline JSON and quotes. So never pass a JSON body on the command line: write it to a file in a temporary directory outside the repository (`mktemp -d` on Linux, `$env:TEMP` on Windows) and pass it with `--in-file`. In PowerShell, write it as UTF-8 without a byte order mark, for example `[IO.File]::WriteAllText($path, $json)`, since `az` reads the file as UTF-8 by default. Delete the files afterwards.

## Where

Take the organization URL, project and repository from `git remote get-url origin`:

- `https://dev.azure.com/<org>/<project>/_git/<repo>` (an `https://<user>@dev.azure.com/...` prefix may be present)
- `git@ssh.dev.azure.com:v3/<org>/<project>/<repo>`
- `https://<org>.visualstudio.com/<project>/_git/<repo>`

The organization URL is `https://dev.azure.com/<org>`. Decode `%20` and other escapes in project and repository names. Pass `--org <org-url> --detect false` to every command so nothing depends on `az devops configure` defaults.

## Watched pull requests

```sh
az repos pr list --org <org-url> --detect false --project <project> --repository <repo> --creator me --status active --top 100 --output json
```

`--creator me` resolves to the signed-in identity. Drop entries with `isDraft: true`. Use `pullRequestId`, `sourceRefName` and `targetRefName` (strip `refs/heads/`), `lastMergeSourceCommit.commitId` as the head commit, `repository.id` as the repository id for the calls below, and `mergeStatus`: `conflicts` means a merge conflict; `queued` means Azure is still computing it, so check again next pass. `createdBy.id` is the user's identity id; use it to recognize the user's comments. A pull request with `forkSource` set comes from a fork: fetch and push with `forkSource.repository.remoteUrl` instead of `origin`.

## Threads

```sh
az devops invoke --org <org-url> --detect false --area git --resource pullRequestThreads --route-parameters project=<project> repositoryId=<repository-id> pullRequestId=<id> --api-version 7.1 --output json
```

The response's `value` lists the threads. Ignore threads with `isDeleted: true` and system threads (every comment has `commentType: "system"`, such as votes and pushes). Watch threads with `status` `active` or `pending`; any other status is already resolved. `threadContext.filePath` is the file a thread is about; a thread without it is a general comment, and on Azure DevOps it can be resolved too. Compare `comments[].author.id` with the user's id. The comment with the highest `id` that is not deleted is the latest.

Reply to a thread with `reply.json` holding `{"content": "<reply>", "parentCommentId": 1, "commentType": 1}` (`1` is the thread's first comment; `commentType` `1` is text):

```sh
az devops invoke --org <org-url> --detect false --area git --resource pullRequestThreadComments --route-parameters project=<project> repositoryId=<repository-id> pullRequestId=<id> threadId=<thread-id> --http-method POST --in-file reply.json --api-version 7.1 --output json
```

Resolve a fixed thread with `status.json` holding `{"status": "fixed"}`:

```sh
az devops invoke --org <org-url> --detect false --area git --resource pullRequestThreads --route-parameters project=<project> repositoryId=<repository-id> pullRequestId=<id> threadId=<thread-id> --http-method PATCH --in-file status.json --api-version 7.1 --output json
```

Leave a declined thread's status `active`.
