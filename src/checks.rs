//! The catalog of Checks a run can go through in the Check stage, as data, and the selection of
//! which ones apply to a diff.
//!
//! A Check applies when the Workflow mode's floor includes it or when a changed path matches one of
//! its triggers. Adding a Check means adding one entry to [`CATALOG`].

use crate::run::Mode;

/// One Check in the catalog.
#[derive(Debug)]
pub(crate) struct Check {
    pub(crate) name: &'static str,
    /// Path globs that make the Check apply when a changed path matches. A pattern without `/`
    /// matches the file name at any depth; `*` stays within a path segment and `**` crosses them.
    pub(crate) triggers: &'static [&'static str],
    /// Modes that always run the Check, whatever the diff touches.
    pub(crate) floor: &'static [Mode],
}

pub(crate) const CATALOG: &[Check] = &[Check {
    name: "code-review",
    triggers: &[],
    floor: &[Mode::Balanced, Mode::Full],
}];

/// Why a Check applies.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Reason {
    /// The mode always runs it.
    Floor,
    /// A changed path matches one of its triggers.
    Triggered { path: String },
}

#[derive(Debug)]
pub(crate) struct Applicable {
    pub(crate) check: &'static Check,
    pub(crate) reason: Reason,
}

#[derive(Debug)]
pub(crate) struct Skipped {
    pub(crate) check: &'static Check,
    pub(crate) reason: String,
}

#[derive(Debug, Default)]
pub(crate) struct Selection {
    pub(crate) applicable: Vec<Applicable>,
    pub(crate) skipped: Vec<Skipped>,
}

/// Splits the catalog into the Checks that apply to `changed_paths` in `mode` and those skipped,
/// each with its reason. `jidoka-fast` has no Check stage, so everything is skipped there.
pub(crate) fn select<S: AsRef<str>>(mode: Mode, changed_paths: &[S]) -> Selection {
    select_from(CATALOG, mode, changed_paths)
}

fn select_from<S: AsRef<str>>(
    catalog: &'static [Check],
    mode: Mode,
    changed_paths: &[S],
) -> Selection {
    let mut selection = Selection::default();
    for check in catalog {
        if mode == Mode::Fast {
            selection.skipped.push(Skipped {
                check,
                reason: format!("{} has no check stage", mode.name()),
            });
        } else if check.floor.contains(&mode) {
            selection.applicable.push(Applicable {
                check,
                reason: Reason::Floor,
            });
        } else if let Some(path) = changed_paths
            .iter()
            .map(AsRef::as_ref)
            .find(|path| check.triggers.iter().any(|glob| matches(glob, path)))
        {
            selection.applicable.push(Applicable {
                check,
                reason: Reason::Triggered {
                    path: path.to_string(),
                },
            });
        } else {
            selection.skipped.push(Skipped {
                check,
                reason: skip_reason(check, mode),
            });
        }
    }
    selection
}

fn skip_reason(check: &Check, mode: Mode) -> String {
    if check.triggers.is_empty() {
        format!("{} does not run it", mode.name())
    } else {
        format!("no changed path matches {}", check.triggers.join(", "))
    }
}

/// Whether `glob` matches `path` (forward slashes, relative to the repository root).
fn matches(glob: &str, path: &str) -> bool {
    if glob.contains('/') {
        glob_match(glob.as_bytes(), path.as_bytes())
    } else {
        let name = path.rsplit('/').next().unwrap_or(path);
        glob_match(glob.as_bytes(), name.as_bytes())
    }
}

fn glob_match(glob: &[u8], text: &[u8]) -> bool {
    match glob {
        [] => text.is_empty(),
        [b'*', b'*', b'/', rest @ ..] => (0..=text.len())
            .any(|skip| (skip == 0 || text[skip - 1] == b'/') && glob_match(rest, &text[skip..])),
        [b'*', b'*', rest @ ..] => (0..=text.len()).any(|skip| glob_match(rest, &text[skip..])),
        [b'*', rest @ ..] => {
            let segment = text
                .iter()
                .position(|byte| *byte == b'/')
                .unwrap_or(text.len());
            (0..=segment).any(|skip| glob_match(rest, &text[skip..]))
        }
        [first, rest @ ..] => text
            .split_first()
            .is_some_and(|(byte, tail)| byte == first && glob_match(rest, tail)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECURITY: Check = Check {
        name: "security",
        triggers: &["Cargo.toml", "*.tf", ".github/workflows/**"],
        floor: &[Mode::Full],
    };
    static EXTENDED: [Check; 2] = [CATALOG_CODE_REVIEW, SECURITY];
    const CATALOG_CODE_REVIEW: Check = Check {
        name: "code-review",
        triggers: &[],
        floor: &[Mode::Balanced, Mode::Full],
    };

    fn names(applicable: &[Applicable]) -> Vec<&'static str> {
        applicable.iter().map(|entry| entry.check.name).collect()
    }

    #[test]
    fn balanced_and_full_always_run_code_review() {
        for mode in [Mode::Balanced, Mode::Full] {
            let selection = select(mode, &["src/main.rs"]);
            assert_eq!(names(&selection.applicable), ["code-review"]);
            assert_eq!(selection.applicable[0].reason, Reason::Floor);
            assert!(selection.skipped.is_empty());
        }
    }

    #[test]
    fn fast_skips_everything_with_a_reason() {
        let selection = select(Mode::Fast, &["src/main.rs"]);
        assert!(selection.applicable.is_empty());
        assert_eq!(selection.skipped.len(), CATALOG.len());
        assert_eq!(
            selection.skipped[0].reason,
            "jidoka-fast has no check stage"
        );
    }

    #[test]
    fn a_matching_path_triggers_a_check_above_the_floor() {
        let selection = select_from(
            &EXTENDED,
            Mode::Balanced,
            &["README.md", ".github/workflows/ci.yml"],
        );
        assert_eq!(names(&selection.applicable), ["code-review", "security"]);
        assert_eq!(
            selection.applicable[1].reason,
            Reason::Triggered {
                path: ".github/workflows/ci.yml".to_string()
            }
        );
    }

    #[test]
    fn an_untriggered_check_is_skipped_with_its_triggers_named() {
        let selection = select_from(&EXTENDED, Mode::Balanced, &["src/main.rs"]);
        assert_eq!(names(&selection.applicable), ["code-review"]);
        assert_eq!(selection.skipped.len(), 1);
        assert_eq!(selection.skipped[0].check.name, "security");
        assert_eq!(
            selection.skipped[0].reason,
            "no changed path matches Cargo.toml, *.tf, .github/workflows/**"
        );
    }

    #[test]
    fn the_floor_applies_without_a_matching_path() {
        let selection = select_from(&EXTENDED, Mode::Full, &[] as &[&str]);
        assert_eq!(names(&selection.applicable), ["code-review", "security"]);
        assert_eq!(selection.applicable[1].reason, Reason::Floor);
    }

    #[test]
    fn globs_match_names_segments_and_trees() {
        assert!(matches("*.tf", "infra/main.tf"));
        assert!(matches("Dockerfile", "svc/Dockerfile"));
        assert!(!matches("Dockerfile", "svc/Dockerfile.md"));
        assert!(matches("dashboard/**", "dashboard/app.js"));
        assert!(matches("dashboard/**", "dashboard/css/app.css"));
        assert!(!matches("dashboard/**", "src/dashboard/app.js"));
        assert!(matches(
            ".github/workflows/*.yml",
            ".github/workflows/ci.yml"
        ));
        assert!(!matches(
            ".github/workflows/*.yml",
            ".github/workflows/a/ci.yml"
        ));
        assert!(matches("**/Cargo.toml", "Cargo.toml"));
        assert!(matches("**/Cargo.toml", "crates/a/Cargo.toml"));
        assert!(!matches("**/Cargo.toml", "myCargo.toml"));
        assert!(!matches("**/Cargo.toml", "crates/fooCargo.toml"));
    }
}
