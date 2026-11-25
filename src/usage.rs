//! Everything Jidoka reads from the agents' session logs: Token usage, the request analysis and attention.
//! The pipeline, in the order the data flows:
//!
//! 1. [`agent_logs`] reads the Claude Code, Codex and Copilot session logs into sessions and responses,
//!    using [`shell_commands`] to find the runs a session's commands name and [`attention`] for the moments a
//!    coordinator waits for its user.
//! 2. [`session_attribution`] finds the sessions a run claims and places each response under the run's
//!    Timeline.
//! 3. Token usage: [`collection`] turns the attributed responses into the counts of [`token_usage`] and the
//!    pauses of [`pauses`], and stores them with the run.
//! 4. Request analysis: [`request_logs`] classifies each model request, [`requests`] reports one run's and
//!    [`requests_pool`] many runs' requests per category.
//! 5. Reports: [`stats`] shows Token usage as text and [`cost`] estimates its cost.

pub(crate) mod agent_logs;
pub(crate) mod attention;
pub(crate) mod collection;
pub(crate) mod cost;
pub(crate) mod pauses;
pub(crate) mod request_logs;
pub(crate) mod requests;
pub(crate) mod requests_pool;
pub(crate) mod session_attribution;
pub(crate) mod shell_commands;
pub(crate) mod stats;
pub(crate) mod token_usage;
