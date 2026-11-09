//! The local Markdown documents (spec and tickets) a run records, read safely and rendered as HTML.

use crate::run::Run;
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf, Prefix},
};

/// The largest document the dashboard renders.
const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;

/// A rendered document: its file name and HTML that is safe to insert into the page.
pub(crate) struct Document {
    pub(crate) name: String,
    pub(crate) html: String,
}

/// Why a document is not served.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DocumentError {
    NotRecorded,
    WebReference,
    NotMarkdown,
    OutsideRoots,
    TooLarge,
    NotText,
    /// The recorded file is not there (yet).
    NotAvailable {
        path: String,
    },
}

/// The recorded local document `reference` of `run`, rendered. The file must be inside the
/// worktree, the repository, the worktree's git directory, or be `<worktree>.factory-plan.md`.
pub(crate) fn read(run: &Run, reference: &str) -> Result<Document, DocumentError> {
    if is_web_reference(reference) {
        return Err(DocumentError::WebReference);
    }
    let recorded = run.spec.as_deref() == Some(reference)
        || run
            .tickets
            .iter()
            .any(|ticket| ticket.reference == reference);
    if !recorded {
        return Err(DocumentError::NotRecorded);
    }
    let reference_path = Path::new(reference);
    require_markdown(reference_path)?;
    let not_available = || DocumentError::NotAvailable {
        path: reference.to_owned(),
    };
    let worktree = run
        .worktree
        .as_deref()
        .map(Path::new)
        .filter(|r| is_git_root(r));
    let repository = run
        .repository
        .as_deref()
        .map(Path::new)
        .filter(|r| is_git_root(r));
    let candidates: Vec<PathBuf> = if reference_path.is_absolute() {
        vec![reference_path.to_path_buf()]
    } else {
        worktree
            .into_iter()
            .chain(repository)
            .map(|root| root.join(reference_path))
            .collect()
    };
    let file = candidates
        .iter()
        .filter(|candidate| is_local_path(candidate))
        .find_map(|candidate| std::fs::canonicalize(candidate).ok())
        .ok_or_else(not_available)?;
    let roots = roots(worktree, repository);
    if !roots.iter().any(|root| file.starts_with(root)) {
        return Err(DocumentError::OutsideRoots);
    }
    require_markdown(&file)?;
    let markdown = read_markdown(&file, &not_available)?;
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Document {
        name,
        html: render_markdown(&markdown),
    })
}

/// Reads the file through one handle, so what is checked is what is read.
fn read_markdown(
    file: &Path,
    not_available: &dyn Fn() -> DocumentError,
) -> Result<String, DocumentError> {
    let handle = File::open(file).map_err(|_| not_available())?;
    let metadata = handle.metadata().map_err(|_| not_available())?;
    if !metadata.is_file() {
        return Err(not_available());
    }
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    let mut bytes = Vec::new();
    handle
        .take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DocumentError::NotText)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| DocumentError::NotText)
}

/// The canonical directories and file a document may live in.
fn roots(worktree: Option<&Path>, repository: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = worktree
        .into_iter()
        .chain(repository)
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    if let Some(worktree) = worktree {
        roots.extend(git_dir(worktree, repository));
        let mut plan = worktree.as_os_str().to_owned();
        plan.push(".factory-plan.md");
        let plan = PathBuf::from(plan);
        if is_local_path(&plan) {
            roots.extend(std::fs::canonicalize(plan).ok());
        }
    }
    roots
}

/// A root that is a real repository or worktree: not a filesystem root, with a `.git` entry, and
/// on a local drive or Unix path.
fn is_git_root(root: &Path) -> bool {
    root.parent().is_some() && is_local_path(root) && root.join(".git").exists()
}

/// Whether `prefix` names a local drive. UNC shares and device paths are not local.
fn is_drive_prefix(prefix: Prefix) -> bool {
    matches!(prefix, Prefix::Disk(_) | Prefix::VerbatimDisk(_))
}

/// Whether `path` stays off network shares and device namespaces; checked before any filesystem access.
fn is_local_path(path: &Path) -> bool {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => is_drive_prefix(prefix.kind()),
        _ => true,
    }
}

/// The canonical git directory of a worktree: `<worktree>/.git` is a directory, or a file
/// `gitdir: <path>` whose target is inside the repository's own `.git` directory.
fn git_dir(worktree: &Path, repository: Option<&Path>) -> Option<PathBuf> {
    let dot_git = worktree.join(".git");
    if dot_git.is_dir() {
        return std::fs::canonicalize(dot_git).ok();
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let target = worktree.join(text.trim().strip_prefix("gitdir:")?.trim());
    if !is_local_path(&target) {
        return None;
    }
    let target = std::fs::canonicalize(target).ok()?;
    let repository_git = std::fs::canonicalize(repository?.join(".git")).ok()?;
    target.starts_with(repository_git).then_some(target)
}

fn require_markdown(path: &Path) -> Result<(), DocumentError> {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        .then_some(())
        .ok_or(DocumentError::NotMarkdown)
}

fn is_web_reference(reference: &str) -> bool {
    let lower = reference.trim_start().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Renders Markdown as HTML that is safe to insert into the page: raw HTML is shown as text, images
/// as their alt text, http(s) links open in a new tab and every other link is plain text.
fn render_markdown(markdown: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut kept_links = Vec::new();
    let events = Parser::new_ext(
        markdown,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    )
    .filter_map(|event| match event {
        Event::Html(html) | Event::InlineHtml(html) => Some(Event::Text(html)),
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
        Event::Start(Tag::Link { dest_url, .. }) => {
            let keep = is_web_reference(&dest_url);
            kept_links.push(keep);
            keep.then(|| {
                Event::Html(
                    format!(
                        "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">",
                        escape_html(&dest_url)
                    )
                    .into(),
                )
            })
        }
        Event::End(TagEnd::Link) => kept_links
            .pop()
            .unwrap_or(false)
            .then(|| Event::Html("</a>".into())),
        other => Some(other),
    });
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events);
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::{ffi::OsStr, fs};

    /// A repository with a `.git` directory and a worktree with its own, and a run recording `spec`.
    fn setup(spec: &str) -> (TestDir, PathBuf, PathBuf, Run) {
        let dir = TestDir::new();
        let repository = dir.0.path().join("repo");
        let worktree = dir.0.path().join("repo.worktrees").join("feature");
        fs::create_dir_all(repository.join(".git")).unwrap();
        fs::create_dir_all(worktree.join(".git")).unwrap();
        let run = run(&repository, &worktree, spec);
        (dir, repository, worktree, run)
    }

    fn run(repository: &Path, worktree: &Path, spec: &str) -> Run {
        serde_json::from_value(serde_json::json!({
            "id": "1", "title": "t", "project": repository.to_string_lossy(),
            "repository": repository.to_string_lossy(), "worktree": worktree.to_string_lossy(),
            "spec": spec, "stage": "build", "status": "running", "tickets": [],
        }))
        .unwrap()
    }

    #[test]
    fn only_drive_prefixes_are_local() {
        assert!(is_drive_prefix(Prefix::Disk(b'C')));
        assert!(is_drive_prefix(Prefix::VerbatimDisk(b'C')));
        assert!(!is_drive_prefix(Prefix::UNC(
            OsStr::new("server"),
            OsStr::new("share")
        )));
        assert!(!is_drive_prefix(Prefix::VerbatimUNC(
            OsStr::new("server"),
            OsStr::new("share")
        )));
        assert!(!is_drive_prefix(Prefix::DeviceNS(OsStr::new("pipe"))));
        assert!(!is_drive_prefix(Prefix::Verbatim(OsStr::new("x"))));
    }

    #[cfg(windows)]
    #[test]
    fn unc_and_device_paths_are_not_local() {
        for path in [
            r"\\server\share\x.md",
            r"\\?\UNC\server\share\x.md",
            r"\\.\pipe\x",
            r"\\?\GLOBALROOT\x.md",
        ] {
            assert!(!is_local_path(Path::new(path)), "{path}");
        }
        assert!(is_local_path(Path::new(r"C:\work\x.md")));
        assert!(is_local_path(Path::new(r"\\?\C:\work\x.md")));
    }

    #[test]
    fn plain_unix_and_relative_paths_are_local() {
        assert!(is_local_path(Path::new("/work/x.md")));
        assert!(is_local_path(Path::new("docs/x.md")));
    }

    #[test]
    fn a_filesystem_root_or_a_directory_without_git_is_not_a_root() {
        let dir = TestDir::new();
        assert!(!is_git_root(Path::new("/")));
        assert!(!is_git_root(dir.0.path()));
        fs::create_dir(dir.0.path().join(".git")).unwrap();
        assert!(is_git_root(dir.0.path()));
    }

    #[test]
    fn a_run_in_a_directory_without_git_serves_nothing() {
        let (_dir, repository, worktree, run) = setup("spec.md");
        fs::write(worktree.join("spec.md"), "x").unwrap();
        assert!(read(&run, "spec.md").is_ok());
        fs::remove_dir(worktree.join(".git")).unwrap();
        fs::remove_dir(repository.join(".git")).unwrap();
        assert_eq!(
            read(&run, "spec.md").err(),
            Some(DocumentError::NotAvailable {
                path: "spec.md".into()
            })
        );
    }

    #[test]
    fn a_gitdir_file_is_trusted_only_inside_the_repository_git_directory() {
        let (dir, repository, worktree, _) = setup("spec.md");
        fs::remove_dir(worktree.join(".git")).unwrap();
        let inside = repository.join(".git/worktrees/feature");
        let outside = dir.0.path().join("elsewhere");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}", inside.display()),
        )
        .unwrap();
        assert_eq!(
            git_dir(&worktree, Some(&repository)),
            Some(fs::canonicalize(&inside).unwrap())
        );
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}", outside.display()),
        )
        .unwrap();
        assert_eq!(git_dir(&worktree, Some(&repository)), None);
    }

    #[test]
    fn a_git_directory_is_used_as_is() {
        let (_dir, repository, worktree, _) = setup("spec.md");
        assert_eq!(
            git_dir(&worktree, Some(&repository)),
            Some(fs::canonicalize(worktree.join(".git")).unwrap())
        );
    }

    #[test]
    fn a_document_over_a_megabyte_is_rejected_but_exactly_a_megabyte_is_served() {
        let (_dir, _, worktree, run) = setup("big.md");
        fs::write(worktree.join("big.md"), vec![b'a'; 1024 * 1024 + 1]).unwrap();
        assert_eq!(read(&run, "big.md").err(), Some(DocumentError::TooLarge));
        fs::write(worktree.join("big.md"), vec![b'a'; 1024 * 1024]).unwrap();
        assert!(read(&run, "big.md").is_ok());
    }

    #[test]
    fn a_file_that_is_not_text_is_rejected() {
        let (_dir, _, worktree, run) = setup("bin.md");
        fs::write(worktree.join("bin.md"), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(read(&run, "bin.md").err(), Some(DocumentError::NotText));
    }

    #[test]
    fn raw_html_is_shown_as_text() {
        let html = render_markdown("<script>alert(1)</script>\n\ntext <b onclick=x>hi</b>");
        assert!(
            !html.contains("<script>") && !html.contains("<b "),
            "{html}"
        );
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn only_http_links_are_links_and_images_are_alt_text() {
        let html = render_markdown(
            "[web](https://example.com/a?b=1&c=2) [rel](other.md) [js](javascript:alert(1)) [mail](mailto:a@b.c) ![alt text](http://evil.test/x.png)",
        );
        assert!(html.contains(
            "<a href=\"https://example.com/a?b=1&amp;c=2\" target=\"_blank\" rel=\"noopener noreferrer\">web</a>"
        ), "{html}");
        assert_eq!(html.matches("<a ").count(), 1, "{html}");
        for text in ["rel", "js", "mail", "alt text"] {
            assert!(html.contains(text), "{html}");
        }
        assert!(
            !html.contains("javascript:") && !html.contains("<img"),
            "{html}"
        );
    }
}
