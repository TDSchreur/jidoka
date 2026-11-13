## Fixed

- `jidoka pr sync` no longer merges the base into a branch whose pull request is already merged or closed. It refuses (exit 1, nothing merged or pushed) when the stored pull request state is merged or closed, or when all of the branch's own commits are already in the base, and points to `jidoka run cleanup` for a merged pull request. Before, it made an empty merge commit and pushed it, after which cleanup refused.

## Changed

- Balanced workflow: Define asks for numbered, individually testable acceptance criteria, and Build starts the runnable entrypoint once with default configuration and ends by listing the evidence for each criterion. Build agents get the saved plan or spec by path or URL.
- `jidoka-code-review` Spec reviewer also starts the entrypoint and checks the main path; its findings are recorded under the `code-review` check, and the unknown-check error now says so.
