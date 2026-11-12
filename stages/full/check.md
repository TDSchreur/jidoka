# Full: Check

Before Check, follow Committing for Check in the shared procedures included below.

Use `jidoka-code-review` with the base commit as the fixed point, and with the spec. Use `jidoka-security-review` on the same range when the change meaningfully affects a security boundary or sensitive data flow, such as authorization or handling secrets or untrusted input across a boundary; a touched file or dependency alone is not sufficient. Otherwise skip it and say why. Honor an explicit user request for a security review. Follow Model routing when supplied for Check: it says which model and effort `jidoka-code-review`'s agents use.

When there are actionable findings, start a Rework: return to Build, fix the findings using the project’s test approach, run appropriate checks, and commit the fixes together with any new files. The coordinator checks small, clear fixes against the findings. For substantial or uncertain fixes, repeat a scoped `jidoka-code-review` from the previously reviewed commit, including the findings addressed; repeat the security review when those fixes meaningfully affect a security boundary or sensitive data flow, or the user requests it. Record how the fixes were checked and pin the new `HEAD`. Follow Reworks in the shared procedures included below.

When no actionable findings remain, record Check's gates; `jidoka run advance` refuses to leave Check until all three pass:

1. Record completed checks with `jidoka run gate <id> checks-recorded pass`.
2. Record `jidoka run gate <id> no-actionable-findings pass`.
3. Run the project's full verification on the clean, final reviewed commit and record it with `jidoka run gate <id> verified pass`. Finish can reuse this result while the checked code and relevant inputs remain unchanged.

Finish puts the review record in the pull request, so keep it short in the Check summary as well: each check that ran with its reviewed commit and outcome, the Reworks, and each skipped check with the reason.
