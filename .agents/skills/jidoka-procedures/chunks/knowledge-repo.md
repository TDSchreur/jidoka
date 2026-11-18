## Knowledge repo

The user keeps a knowledge repo: an ordinary repository with the platform's shared knowledge (`platform/`), recipes (`recipes/`), learnings (`learnings/`) and cross-repo change specs (`changes/`). It is **not** worked through Jidoka runs: it needs no run, worktree or pull request. Its checkout is printed above. This applies to every repository the run touches.

**Define.** Before the spec, run `git pull --rebase` in the knowledge repo, read `platform/` and search `recipes/` and `learnings/` for the topic. Use what is relevant for the spec and mention it in the spec; skip folders that do not exist.

**Finish.** After the pull request is open, record what this change taught about the platform, if anything, under the rules below. Say in the Finish report what you recorded, or that nothing qualified.

Rules:

- Record a **durable fact about the platform** that was not already in the knowledge repo: how repositories relate, deployment order, naming, conventions, gotchas, ownership. Do not record task status, speculation, or facts about one repository that its own docs already hold.
- Read first. Search the knowledge repo before writing. Update an existing note instead of adding a duplicate, and correct a note that turned out wrong.
- One fact per note or section, with the date and its source (repository and pull request or file). Keep it short.
- Never record secrets, tokens, credentials, personal data or private customer data.
- Write directly: `git pull --rebase`, one note per commit with a clear message, then push to the default branch. If the push is rejected, pull with rebase again and retry; resolve a real conflict by reading both sides, never by discarding another agent's text.
- When unsure that a fact is durable, put it in `learnings/` marked `unverified`. A later review promotes it to `platform/` or `recipes/`, or deletes it.
