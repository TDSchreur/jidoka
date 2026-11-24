## Check reviews

Use `jidoka-security-review` on the same range as the code review when the change meaningfully affects a security boundary or sensitive data flow, such as authorization, or handling secrets or untrusted input across a boundary; a touched file or dependency alone is not sufficient. Otherwise skip it and say why. Honor an explicit user request for a security review. Follow Model routing when supplied for Check: it says which model and effort `jidoka-code-review`'s agents use.

Record every issue a check reports as a Finding, those you will fix and those you judge not worth fixing: `jidoka run update <id> --finding <check> <location|-> "<text>"` (repeat `--finding`; `<check>` is a name listed by `run next`, or `security` for `jidoka-security-review`, `<location>` is a file and line or `-`). Findings from the Standards and Spec reviews of `jidoka-code-review` go under the `code-review` check. Jidoka records the reviewed commit. Record only what the checks reported; do not invent findings. Resolve each Finding that is not being fixed in Check with `--finding-status <finding-id>=dismissed:<reason>` (not a real problem) or `<finding-id>=deferred:<reason>` (a real problem left unfixed on purpose); a reason is required for both.

Wait until every check you started in Check (for example both the Standards and Spec reviews of `jidoka-code-review`, and a security review) has reported, and record all their findings, before starting a Rework. A check result that still arrives after the Rework started is recorded in Build with `jidoka run update <id> --finding <check> <location|-> "<text>"`; it joins the current Rework in Build. The recorded Stage exit keeps the Findings and classifications from the time the Rework started. If the current Rework is Rework 3 classified `wording` (with no recorded decision for behavior fixes) and the late finding needs a behavior fix, leave it open for the next Check to seek the user's decision before fixing it.

When there are actionable findings, start a Rework with `jidoka run rework` with one classification for each open Finding (see Reworks). Back in Build, apply the workflow’s build approach, run appropriate checks and commit the fixes together with any new files. The coordinator checks small, clear fixes against the findings. For substantial or uncertain fixes, repeat a scoped `jidoka-code-review` from the previously reviewed commit, including the findings addressed; repeat the security review when those fixes meaningfully affect a security boundary or sensitive data flow, or the user requests it. Record how the fixes were checked and pin the new `HEAD`. Follow Reworks in these shared procedures.

When no actionable findings remain, record Check's gates; `jidoka run advance` refuses to leave Check until all three pass, and `no-actionable-findings` is refused while any Finding is open (see `jidoka run inspect`):

1. Record each completed check with `jidoka run gate <id> checks-recorded pass --check <name>`; use the names listed by `run next`.
2. Record `jidoka run gate <id> no-actionable-findings pass`.
3. Run the project's full verification on the clean, final reviewed commit and record it with `jidoka run gate <id> verified pass`. Finish can reuse this result while the checked code and relevant inputs remain unchanged.

Once the verification has passed, run the gate commands and `jidoka run advance` as one shell command joined with `&&`, so recording them costs a single turn.

Finish puts the review record in the pull request, so keep it short in the Check summary as well: each check that ran with its reviewed commit and outcome, the Reworks, the deferred Findings, and each skipped check with the reason.
