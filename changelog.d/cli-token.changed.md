## Changed

- POST requests to the dashboard now need a CLI token that `factory` reads from a user-only file in the data folder (`session-token`, or `session-token-<port>` for another port). A missing or wrong token gets 401, and the dashboard exits if it cannot write the token file. Restart the dashboard with the updated CLI after upgrading.
- The CLI reads at most a few bytes of the token file and treats a token file that is not a regular file, or is longer than a token, like a missing one: it says to start or restart `factory dashboard`. Only lowercase hex tokens, as the dashboard writes them, are accepted.
