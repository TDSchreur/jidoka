## Changed

- The factory workflows load the Humanizer skill right before a run writes its first human-facing text, such as a spec, plan or commit message, and no longer when the run starts. Every text still gets the Humanizer pass.
