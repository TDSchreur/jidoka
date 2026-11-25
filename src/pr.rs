//! Pull requests. `host` reads and writes them on GitHub and Azure DevOps; `workspace` holds the checks that
//! `pr open` and `pr sync` share; `open`, `sync` and `wait` are the `jidoka pr` commands; and `state` is the
//! pull request state a Run follows, which `run sync` and the dashboard read.

pub(crate) mod host;
pub(crate) mod open;
pub(crate) mod state;
pub(crate) mod sync;
pub(crate) mod wait;
pub(crate) mod workspace;
