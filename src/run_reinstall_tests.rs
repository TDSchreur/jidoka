//! The reinstall offer with real git in temporary repositories, a scripted `gh` and a recording runner.

use super::*;
use crate::{
    pr_sync::RealGit,
    pr_wait::{GhError, GhOutput},
};
use std::process::Command as Process;
use tempfile::TempDir;

const PR: &str = "https://github.com/o/r/pull/7";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("factory-tests-no-such-gitconfig"),
        )
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A `gh` that reports `merge` as the merge commit, or fails when there is none.
struct FakeGh(Option<String>);

impl Gh for FakeGh {
    fn run(&self, args: &[&str]) -> Result<GhOutput, GhError> {
        if !args.contains(&"mergeCommit") {
            return Ok(GhOutput {
                code: Some(0),
                stdout: r#"{"state":"MERGED","isDraft":false}"#.into(),
                stderr: String::new(),
            });
        }
        Ok(GhOutput {
            code: self.0.as_ref().map(|_| 0),
            stdout: format!(
                r#"{{"mergeCommit":{{"oid":"{}"}}}}"#,
                self.0.clone().unwrap_or_default()
            ),
            stderr: "boom".into(),
        })
    }
}

#[derive(Default)]
struct Recorder {
    said: String,
    asked: Vec<String>,
    answer: bool,
    ran: Vec<Command>,
    fail: bool,
}

impl Io for Recorder {
    fn say(&mut self, text: &str) {
        self.said.push_str(text);
    }
    fn confirm(&mut self, question: &str) -> bool {
        self.asked.push(question.into());
        self.answer
    }
    fn run(&mut self, command: &Command) -> Result<(), String> {
        self.ran.push(command.clone());
        if self.fail && command.program == "factory" {
            Err("exit status 1".into())
        } else {
            Ok(())
        }
    }
}

struct Fixture {
    _tmp: TempDir,
    repo: PathBuf,
    head: String,
    /// A commit that HEAD does not contain.
    other: String,
    windows: std::cell::Cell<bool>,
}

impl Fixture {
    fn new(factory: bool) -> Self {
        Self::named(factory, "repo")
    }

    fn named(factory: bool, name: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join(name);
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "develop"]);
        if factory {
            fs::write(repo.join("Cargo.toml"), "[package]\nname = \"factory\"\n").unwrap();
            let skill = repo.join(".agents/skills/factory-procedures");
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), "x").unwrap();
        } else {
            fs::write(repo.join("Cargo.toml"), "[package]\nname = \"other\"\n").unwrap();
        }
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["checkout", "-q", "-b", "side"]);
        fs::write(repo.join("s.txt"), "s").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "side"]);
        let other = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["checkout", "-q", "develop"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        Fixture {
            _tmp: tmp,
            repo,
            head,
            other,
            windows: Default::default(),
        }
    }

    fn offer(
        &self,
        merge: Option<&str>,
        agents: &[&'static str],
        tty: bool,
        reinstall: bool,
        io: &mut Recorder,
    ) -> Result<(), ToolProblem> {
        let dirs = [PathBuf::from("/c")];
        let input = Inputs {
            repository: &self.repo,
            pr: PR,
            agents,
            claude_dirs: &dirs,
            tty,
            reinstall,
            windows: self.windows.get(),
        };
        offer(&input, &FakeGh(merge.map(str::to_string)), &RealGit, io)
    }
}

fn args(command: &Command) -> Vec<String> {
    command
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn another_repository_gets_no_offer() {
    let fx = Fixture::new(false);
    let mut io = Recorder::default();
    fx.offer(Some(&fx.head), &["claude"], true, true, &mut io)
        .unwrap();
    assert!(io.said.is_empty() && io.asked.is_empty() && io.ran.is_empty());
}

#[test]
fn off_a_terminal_the_commands_are_printed_and_nothing_runs() {
    let fx = Fixture::new(true);
    let mut io = Recorder::default();
    fx.offer(Some(&fx.head), &["codex", "claude"], false, false, &mut io)
        .unwrap();
    assert!(io.ran.is_empty() && io.asked.is_empty());
    assert!(io.said.contains(&format!(
        "cargo install --path {} --locked --force",
        fx.repo.display()
    )));
    assert!(io
        .said
        .contains("factory install --agent codex,claude --config-dir /c"));
}

#[test]
fn yes_on_a_terminal_runs_both_commands_in_order() {
    let fx = Fixture::new(true);
    let mut io = Recorder {
        answer: true,
        ..Default::default()
    };
    fx.offer(Some(&fx.head), &["claude"], true, false, &mut io)
        .unwrap();
    assert_eq!(
        io.asked,
        [format!(
            "Reinstall the factory for claude from {}? [y/N]",
            fx.repo.display()
        )]
    );
    assert!(
        io.said.contains("cargo install"),
        "commands come before the question"
    );
    let [cargo, factory] = &io.ran[..] else {
        panic!("{:?}", io.ran)
    };
    assert_eq!(cargo.program, "cargo");
    assert_eq!(
        args(cargo),
        [
            "install",
            "--path",
            fx.repo.to_str().unwrap(),
            "--locked",
            "--force"
        ]
    );
    assert_eq!(factory.program, "factory");
    assert_eq!(
        args(factory),
        ["install", "--agent", "claude", "--config-dir", "/c"]
    );
    assert_eq!((&cargo.dir, &factory.dir), (&fx.repo, &fx.repo));
    assert!(io.said.contains("factory dashboard"));
}

#[test]
fn no_on_a_terminal_runs_nothing() {
    let fx = Fixture::new(true);
    let mut io = Recorder::default();
    fx.offer(Some(&fx.head), &["claude"], true, false, &mut io)
        .unwrap();
    assert_eq!(io.asked.len(), 1);
    assert!(io.ran.is_empty());
}

#[test]
fn the_flag_reinstalls_without_asking_and_skips_config_dirs_without_claude() {
    let fx = Fixture::new(true);
    let mut io = Recorder::default();
    fx.offer(Some(&fx.head), &["codex"], false, true, &mut io)
        .unwrap();
    assert!(io.asked.is_empty());
    assert_eq!(io.ran.len(), 2);
    assert_eq!(args(&io.ran[1]), ["install", "--agent", "codex"]);
}

#[test]
fn a_merge_that_is_not_in_the_checkout_gives_the_pull_hint() {
    let fx = Fixture::new(true);
    for merge in [Some(fx.other.as_str()), None] {
        let mut io = Recorder::default();
        fx.offer(merge, &["claude"], true, true, &mut io).unwrap();
        assert!(io.ran.is_empty() && io.asked.is_empty());
        assert!(io.said.contains("Pull the base branch"));
        assert!(io.said.contains("cargo install"));
    }
}

#[test]
fn a_pull_request_that_is_not_merged_gets_no_offer() {
    struct Open;
    impl Gh for Open {
        fn run(&self, _args: &[&str]) -> Result<GhOutput, GhError> {
            Ok(GhOutput {
                code: Some(0),
                stdout: r#"{"state":"OPEN","isDraft":false}"#.into(),
                stderr: String::new(),
            })
        }
    }
    let fx = Fixture::new(true);
    let mut io = Recorder::default();
    let input = Inputs {
        repository: &fx.repo,
        pr: PR,
        agents: &["claude"],
        claude_dirs: &[],
        tty: true,
        reinstall: true,
        windows: false,
    };
    offer(&input, &Open, &RealGit, &mut io).unwrap();
    assert!(io.said.is_empty() && io.ran.is_empty());
}

#[test]
fn no_agents_says_so_and_runs_nothing() {
    let fx = Fixture::new(true);
    let mut io = Recorder::default();
    fx.offer(Some(&fx.head), &[], true, true, &mut io).unwrap();
    assert!(io.said.contains("not installed for any agent"));
    assert!(io.ran.is_empty() && io.asked.is_empty());
}

#[test]
fn a_failing_command_is_named_and_the_cleanup_is_said_to_have_succeeded() {
    let fx = Fixture::new(true);
    let mut io = Recorder {
        fail: true,
        ..Default::default()
    };
    let error = fx
        .offer(Some(&fx.head), &["claude"], false, true, &mut io)
        .unwrap_err();
    assert!(error.0.contains("factory install --agent claude"));
    assert!(error.0.contains("cleanup itself succeeded"));
    assert!(!io.said.contains("factory dashboard"));
}

#[test]
fn printed_commands_quote_a_path_with_a_space() {
    let fx = Fixture::named(true, "my repo");
    let path = fx.repo.display().to_string();
    let mut io = Recorder::default();
    fx.offer(Some(&fx.head), &["claude"], false, false, &mut io)
        .unwrap();
    assert!(
        io.said.contains(&format!("--path '{path}' --locked")),
        "{}",
        io.said
    );
    let mut io = Recorder::default();
    fx.offer(Some(&fx.other), &["claude"], false, false, &mut io)
        .unwrap();
    assert!(
        io.said.contains(&format!("git -C '{path}' pull")),
        "{}",
        io.said
    );
    assert_eq!(quote("it's", false), r"'it'\''s'");
    assert_eq!(quote("", false), "''");
    assert_eq!(quote("--locked", false), "--locked");
}

#[test]
fn control_characters_in_a_path_never_reach_the_terminal() {
    let fx = Fixture::named(true, "re\u{1b}[31mpo");
    for (tty, merge) in [(false, &fx.head), (true, &fx.head), (false, &fx.other)] {
        let mut io = Recorder::default();
        fx.offer(Some(merge), &["claude"], tty, false, &mut io)
            .unwrap();
        let all = format!("{}{}", io.said, io.asked.concat());
        assert!(!all.contains('\u{1b}') && all.contains("po"), "{all:?}");
    }
    let mut io = Recorder {
        fail: true,
        ..Default::default()
    };
    let error = fx
        .offer(Some(&fx.head), &["claude"], false, true, &mut io)
        .unwrap_err();
    assert!(!error.0.contains('\u{1b}'));
}

#[test]
fn on_windows_the_commands_are_printed_and_never_run() {
    let fx = Fixture::new(true);
    fx.windows.set(true);
    for (tty, reinstall) in [(true, true), (false, false), (true, false)] {
        let mut io = Recorder {
            answer: true,
            ..Default::default()
        };
        fx.offer(Some(&fx.head), &["claude"], tty, reinstall, &mut io)
            .unwrap();
        assert!(io.ran.is_empty() && io.asked.is_empty());
        assert!(io.said.contains("after this command exits"), "{}", io.said);
        assert!(io.said.contains("cargo install"));
    }
}

#[test]
fn on_windows_printed_commands_use_double_quotes() {
    let fx = Fixture::named(true, "my repo");
    fx.windows.set(true);
    let path = fx.repo.display().to_string();
    let mut io = Recorder::default();
    fx.offer(Some(&fx.head), &["claude"], false, false, &mut io)
        .unwrap();
    assert!(
        io.said.contains(&format!("--path \"{path}\" --locked")),
        "{}",
        io.said
    );
    let mut io = Recorder::default();
    fx.offer(Some(&fx.other), &["claude"], false, false, &mut io)
        .unwrap();
    assert!(
        io.said.contains(&format!("git -C \"{path}\" pull")),
        "{}",
        io.said
    );
    assert!(!io.said.contains('\''), "{}", io.said);
}

/// No line that could be run holds `unsafe_text`: a line that starts with a program name must not carry it.
fn runnable_lines_with<'a>(said: &'a str, unsafe_text: &str) -> Vec<&'a str> {
    said.lines()
        .filter(|l| {
            l.trim_start().starts_with("cargo ")
                || l.trim_start().starts_with("factory ")
                || l.trim_start().starts_with("git ")
        })
        .filter(|l| l.contains(unsafe_text))
        .collect()
}

#[test]
fn on_windows_a_path_the_shell_would_expand_gets_no_pasteable_command() {
    for (name, text) in [("a$(calc)b", "$(calc)"), ("%PATH%x", "%PATH%")] {
        let fx = Fixture::named(true, name);
        fx.windows.set(true);
        for merge in [&fx.head, &fx.other] {
            let mut io = Recorder::default();
            fx.offer(Some(merge), &["claude"], false, false, &mut io)
                .unwrap();
            assert!(
                runnable_lines_with(&io.said, text).is_empty(),
                "{}",
                io.said
            );
            assert!(io.said.contains("by hand"), "{}", io.said);
        }
    }
    assert_eq!(quote("50%", true), "\"50%\"");
}
