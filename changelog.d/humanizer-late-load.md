## Changed

- The factory workflows load the Humanizer skill right before a run writes its first spec, ticket, plan, summary, commit message or pull request description, instead of at the start of the run. Every text still gets the Humanizer pass.
