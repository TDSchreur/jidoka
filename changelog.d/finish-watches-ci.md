## Changed

- The Finish stage of `/factory`, `/factory-balanced` and `/factory-fast` now watches the pull request's CI after opening it and fixes failing checks with new commits until CI is green. The run is reported done, and the pull request ready for review, only then. Without CI the watch is skipped, and the agent never merges.
