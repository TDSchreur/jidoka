## Changed

- Pull request descriptions written by `jidoka-pr` have a Review section: the reviews that ran, with their commit and outcome, the skipped ones with the reason, and any remaining findings. It replaces the Remaining review findings section, and Finish passes the Check results to it.
- Check instructions list the three gates to record before `jidoka run advance`, instead of saying "go to Finish" first.
- The workflow guide says the security review is decided per change in Balanced and Full; `jidoka run next` lists only the code review.

## Removed

- The workflow selector no longer mentions the legacy `jidoka` mode, which runs no longer record.
