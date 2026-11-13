## Check reviews

Use `jidoka-security-review` on the same range as the code review when the change meaningfully affects a security boundary or sensitive data flow, such as authorization, or handling secrets or untrusted input across a boundary; a touched file or dependency alone is not sufficient. Otherwise skip it and say why. Honor an explicit user request for a security review. Follow Model routing when supplied for Check: it says which model and effort `jidoka-code-review`'s agents use.

Record every issue a check reports as a Finding, those you will fix and those you judge not worth fixing: `jidoka run update <id> running --finding <check> <location|-> "<text>"` (repeat `--finding`; `<check>` is a name listed by `run next`, or `security` for `jidoka-security-review`, `<location>` is a file and line or `-`). Jidoka records the reviewed commit. Record only what the checks reported; do not invent findings. Resolve each Finding that is not being fixed in Check with `--finding-status <finding-id>=dismissed:<reason>` (not a real problem) or `<finding-id>=deferred:<reason>` (a real problem left unfixed on purpose); a reason is required for both.

When there are actionable findings, start a Rework with the findings in the same update (see Reworks): return to Build, fix the findings as the workflow says above, run appropriate checks, and commit the fixes together with any new files. The coordinator checks small, clear fixes against the findings. For substantial or uncertain fixes, repeat a scoped `jidoka-code-review` from the previously reviewed commit, including the findings addressed; repeat the security review when those fixes meaningfully affect a security boundary or sensitive data flow, or the user requests it. Record how the fixes were checked and pin the new `HEAD`. Follow Reworks in these shared procedures.

When no actionable findings remain, record Check's gates; `jidoka run advance` refuses to leave Check until all three pass, and `no-actionable-findings` is refused while any Finding is open (see `jidoka run inspect`):

1. Record each completed check with `jidoka run gate <id> checks-recorded pass --check <name>`; use the names listed by `run next`.
2. Record `jidoka run gate <id> no-actionable-findings pass`.
3. Run the project's full verification on the clean, final reviewed commit and record it with `jidoka run gate <id> verified pass`. Finish can reuse this result while the checked code and relevant inputs remain unchanged.

Finish puts the review record in the pull request, so keep it short in the Check summary as well: each check that ran with its reviewed commit and outcome, the Reworks, the deferred Findings, and each skipped check with the reason.
