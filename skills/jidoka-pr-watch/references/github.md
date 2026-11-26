# GitHub commands

Commands for [jidoka-pr-watch](../SKILL.md) on GitHub, using the `gh` CLI signed in as the user. They work the same in Bash and PowerShell. Pass every comment body and GraphQL query through a file (`--body-file`, `-F name=@file`) instead of inline quoting, because multi-line text and quotes break differently in each shell. Write those files to a temporary directory outside the repository (`mktemp -d` on Linux, `$env:TEMP` on Windows) and delete them afterwards.

## Who and where

- Signed-in user: `gh api user --jq .login`.
- Repository: `gh repo view --json owner,name`.

## Watched pull requests

```sh
gh pr list --author @me --state open --limit 100 --json number,url,isDraft,headRefName,headRefOid,baseRefName,mergeable,mergeStateStatus,isCrossRepository,headRepositoryOwner,headRepository
```

Drop entries with `isDraft: true`. `mergeable` is `CONFLICTING` for a merge conflict; `UNKNOWN` means GitHub is still computing it, so check again next pass instead of rebasing. For `isCrossRepository: true`, the branch lives in `<headRepositoryOwner.login>/<headRepository.name>`: fetch and push with that repository's URL instead of `origin`.

## Review threads

Save this query as `threads.graphql`:

```graphql
query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 50, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          comments(last: 50) {
            nodes { author { login __typename } body createdAt url }
          }
        }
      }
    }
  }
}
```

```sh
gh api graphql -F query=@threads.graphql -f owner=<owner> -f name=<name> -F number=<number>
```

Page with `-f after=<endCursor>` while `hasNextPage` is true. Keep threads with `isResolved: false`. `comments(last: 50)` returns the newest comments, so the last node is always the latest comment, even on a longer thread; an `author.__typename` of `Bot` marks a bot.

Reply to a thread (body in `reply.md`), saving the mutation as `reply.graphql`:

```graphql
mutation($thread: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: {pullRequestReviewThreadId: $thread, body: $body}) {
    comment { url }
  }
}
```

```sh
gh api graphql -F query=@reply.graphql -f thread=<thread-id> -F body=@reply.md
```

Resolve a fixed thread, saving the mutation as `resolve.graphql`:

```graphql
mutation($thread: ID!) {
  resolveReviewThread(input: {threadId: $thread}) {
    thread { isResolved }
  }
}
```

```sh
gh api graphql -F query=@resolve.graphql -f thread=<thread-id>
```

## Conversation comments and review bodies

```sh
gh pr view <number> --json comments,reviews
```

`comments` are the top-level conversation; `reviews` carry each review's `body` and `state`. Ignore reviews with an empty body: their content is in the review threads above. Neither can be resolved. Answer one with:

```sh
gh pr comment <number> --body-file reply.md
```
