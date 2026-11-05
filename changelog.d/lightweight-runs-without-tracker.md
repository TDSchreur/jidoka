## Changed

- `/factory-balanced` and `/factory-fast` no longer stop for tracker setup when a clear, bounded request needs no spec or tickets in the tracker. They still require it before reading or publishing a spec or tickets, and never guess a tracker; `/factory` is unchanged.
- Those runs now save their plan and acceptance criteria to a file outside the pull request before Build and record it with `--spec`, so a new session can continue from `factory run inspect`. `factory doctor` and the README explain which workflow needs the tracker setup.
