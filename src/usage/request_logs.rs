//! Categories of model requests for `jidoka run requests`, read from the shell commands of their tool calls.
//!
//! Of each request it keeps the token counts and a category taken from the Bash commands of its
//! tool calls. The command text is dropped as soon as it is classified: nothing but categories, flags and
//! counts leaves this module.

use crate::usage::{
    agent_logs::Response,
    shell_commands::{command_words, segments, skip_git_options},
    token_usage::Tokens,
};
use serde::Serialize;
use std::collections::BTreeSet;

/// What a model request did. The order is the priority: a request that fits several categories takes the
/// first one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Category {
    Workspace,
    Commit,
    PrCreationAndPush,
    PrSynchronization,
    CiWatch,
    Verification,
    ContinuationCheck,
    /// Its only command updates the run.
    StandaloneReporting,
    /// Any other tool call.
    ReadingEditingAnalysis,
    /// No tool call.
    TextOnly,
    /// A Copilot request whose log has no assistant message to read its tool calls from.
    NotCategorized,
}

impl Category {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Category::Workspace => "workspace",
            Category::Commit => "commit",
            Category::PrCreationAndPush => "PR creation and push",
            Category::PrSynchronization => "PR synchronization",
            Category::CiWatch => "CI watch",
            Category::Verification => "verification",
            Category::ContinuationCheck => "continuation check",
            Category::StandaloneReporting => "standalone reporting",
            Category::ReadingEditingAnalysis => "reading, editing, analysis",
            Category::TextOnly => "text only",
            Category::NotCategorized => "not categorized",
        }
    }
}

/// One model request.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) category: Category,
    pub(crate) tokens: Tokens,
    /// Its commands also updated the run, next to other work.
    pub(crate) chained_reporting: bool,
    /// Its commands fit more than one category.
    pub(crate) several_categories: bool,
    /// The bytes of text its tool calls returned; unknown when the log does not say.
    pub(crate) tool_output_bytes: Option<u64>,
    /// Whether a subagent made it.
    pub(crate) subagent: bool,
}

/// The request a response made: its category comes from the commands of its tool calls.
pub(crate) fn request_of(response: &Response) -> Request {
    // A response whose tool calls the log does not give is not known to have made none.
    let class = if response.calls_known {
        classify(&response.commands, response.other_tool, response.any_tool)
    } else {
        Classification {
            category: Category::NotCategorized,
            chained_reporting: false,
            several_categories: false,
        }
    };
    Request {
        category: class.category,
        tokens: response.tokens,
        chained_reporting: class.chained_reporting,
        several_categories: class.several_categories,
        tool_output_bytes: response.tool_output_bytes,
        subagent: response.subagent,
    }
}

// --- categories -------------------------------------------------------------------------------------

/// Programs that only read or reformat text. In a substitution or after a pipe they support the command they
/// belong to; any other program there (`git apply`, `xargs`, `tee`, a script) does work of its own.
fn is_read_only_helper(segment: &str) -> bool {
    const HELPERS: [&str; 20] = [
        "cat", "tail", "head", "grep", "egrep", "fgrep", "rg", "wc", "sort", "uniq", "cut", "tr",
        "jq", "column", "date", "basename", "dirname", "pwd", "whoami", "realpath",
    ];
    let mut words = command_words(segment);
    let program = words
        .first()
        .map(|word| word.rsplit('/').next().unwrap_or(word));
    if program == Some("git") {
        skip_git_options(&mut words);
    }
    let word = |index: usize| words.get(index).copied().unwrap_or("");
    // `git diff --output=<file>` writes a file.
    if words.iter().any(|word| word.starts_with("--output")) {
        return false;
    }
    program.is_some_and(|program| HELPERS.contains(&program))
        || matches!(
            (program, word(1), word(2)),
            (
                Some("git"),
                "rev-parse" | "log" | "status" | "diff" | "show",
                _
            ) | (Some("git"), "branch", "--show-current")
                | (Some("gh"), "pr" | "issue" | "repo", "view")
        )
}

/// What one command of a shell line stands for.
enum Hit {
    Category(Category),
    Reporting,
    /// `cd`, `echo` and the like: not a command that did the work.
    Neutral,
    Other,
}

/// How a request was classified.
pub(crate) struct Classification {
    pub(crate) category: Category,
    pub(crate) chained_reporting: bool,
    pub(crate) several_categories: bool,
}

/// The category of a request from its Bash commands, whether it called any other tool, and whether it
/// called any tool at all.
pub(crate) fn classify(commands: &[String], other_tool: bool, any_tool: bool) -> Classification {
    if !any_tool {
        return Classification {
            category: Category::TextOnly,
            chained_reporting: false,
            several_categories: false,
        };
    }
    let mut categories: BTreeSet<Category> = BTreeSet::new();
    let (mut reporting, mut other) = (false, other_tool);
    for command in commands {
        for segment in segments(command) {
            match hit(&segment.text) {
                Hit::Category(category) => {
                    categories.insert(category);
                }
                Hit::Reporting => reporting = true,
                Hit::Neutral => {}
                // A read-only helper, such as `cat file` in a substitution or `tail -1` after a pipe, is
                // not work of its own. Anything else that runs there is.
                Hit::Other if segment.plumbing && is_read_only_helper(&segment.text) => {}
                Hit::Other => other = true,
            }
        }
    }
    let category = if let Some(first) = categories.first() {
        *first
    } else if reporting && !other {
        Category::StandaloneReporting
    } else {
        Category::ReadingEditingAnalysis
    };
    Classification {
        category,
        chained_reporting: reporting && category != Category::StandaloneReporting,
        several_categories: categories.len() > 1,
    }
}

fn hit(segment: &str) -> Hit {
    let mut words = command_words(segment);
    let Some(program) = words
        .first()
        .map(|word| word.rsplit('/').next().unwrap_or(word))
    else {
        return Hit::Neutral;
    };
    if program == "git" {
        skip_git_options(&mut words);
    }
    let word = |index: usize| words.get(index).copied().unwrap_or("");
    let has = |text: &str| words.iter().any(|w| w.contains(text));
    let category = match (program, word(1), word(2)) {
        ("cd" | "pushd" | "popd" | "echo" | "export" | "set" | "true" | ":" | "printf", _, _) => {
            return Hit::Neutral;
        }
        ("git", "worktree", "list") => Category::ContinuationCheck,
        ("git", "worktree" | "clone" | "symbolic-ref", _) => Category::Workspace,
        ("git", "commit" | "add", _) => Category::Commit,
        ("git", "push", _) => Category::PrCreationAndPush,
        ("git", "fetch" | "merge" | "merge-base" | "pull", _) => Category::PrSynchronization,
        ("git", "rev-parse", _) if has("--git-common-dir") => Category::ContinuationCheck,
        ("gh", "pr", "create" | "edit") => Category::PrCreationAndPush,
        ("gh", "pr", "checks") | ("gh", "run", "watch" | "view" | "list") => Category::CiWatch,
        ("gh", "pr", "view") if has("mergeStateStatus") => Category::PrSynchronization,
        ("gh", "api", _) if has("actions/runs") || has("check-runs") => Category::CiWatch,
        ("az", "repos", "pr") if matches!(word(3), "create" | "update") => {
            Category::PrCreationAndPush
        }
        ("az", "repos", "pr") if word(3) == "policy" && word(4) == "list" => Category::CiWatch,
        ("az", "pipelines", "runs") => Category::CiWatch,
        // `pr wait` reads checks with `--resource pullRequestStatuses` or `pullRequestIterationStatuses`.
        ("az", "devops", "invoke")
            if words
                .iter()
                .any(|w| matches!(*w, "pullRequestStatuses" | "pullRequestIterationStatuses")) =>
        {
            Category::CiWatch
        }
        ("jidoka", "run", "start") => Category::Workspace,
        ("jidoka", "run", "update") => return Hit::Reporting,
        ("jidoka", "run", "inspect" | "check") => Category::ContinuationCheck,
        ("cargo", "test" | "fmt" | "clippy" | "build" | "check" | "nextest", _) => {
            Category::Verification
        }
        ("rustfmt" | "pytest", _, _) => Category::Verification,
        ("npm" | "pnpm" | "yarn" | "bun", "test" | "lint" | "build", _) => Category::Verification,
        ("npm" | "pnpm" | "yarn" | "bun", "run", "test" | "lint" | "build" | "check") => {
            Category::Verification
        }
        ("go", "test" | "vet" | "build", _) => Category::Verification,
        ("dotnet", "test" | "build", _) => Category::Verification,
        ("make", "test" | "check" | "lint", _) => Category::Verification,
        ("sh" | "bash" | "zsh", script, _) if script.ends_with("verify.sh") => {
            Category::Verification
        }
        (script, _, _) if script.ends_with("verify.sh") => Category::Verification,
        _ => return Hit::Other,
    };
    Hit::Category(category)
}
