## Added

- The dashboard shows the state of each Run's pull request next to its link: Open, Draft, Merged or Closed. The dashboard server reads it from GitHub with `gh` in the background, keeps it on the Run, and reads it again about once a minute until the pull request is merged. The Run's detail says when it was last checked, or why it could not be read.
