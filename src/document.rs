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
#[derive(Clone, Debug, PartialEq, Eq)]
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
/// worktree or the repository, or be exactly `plan`, the run's own plan file (never its folder).
pub(crate) fn read(run: &Run, reference: &str, plan: &Path) -> Result<Document, DocumentError> {
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
    let not_available = DocumentError::NotAvailable {
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
        .ok_or_else(|| not_available.clone())?;
    let roots = roots(worktree, repository);
    if !roots.iter().any(|root| file.starts_with(root)) && !is_own_plan(&file, plan) {
        return Err(DocumentError::OutsideRoots);
    }
    require_markdown(&file)?;
    if roots.iter().any(|root| file.starts_with(root.join(".git"))) {
        return Err(DocumentError::OutsideRoots);
    }
    let markdown = read_markdown(&file, not_available)?;
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Document {
        name,
        html: render_markdown(&markdown),
    })
}

/// Reads the file through one handle, so what is checked is what is read. `not_available` is the
/// error for a file that is not a readable regular file.
fn read_markdown(file: &Path, not_available: DocumentError) -> Result<String, DocumentError> {
    // Opening a FIFO or device can block forever, so only a regular file is opened.
    if !std::fs::metadata(file).is_ok_and(|metadata| metadata.is_file()) {
        return Err(not_available);
    }
    // Accepted: a symlink swapped in between the canonicalize and this open is not detected. That
    // needs write access inside the run's worktree or repository, and the dashboard is local-only.
    let handle = File::open(file).map_err(|_| not_available.clone())?;
    let metadata = handle.metadata().map_err(|_| not_available.clone())?;
    if !metadata.is_file() {
        return Err(not_available);
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

/// The canonical directories a document may live in.
fn roots(worktree: Option<&Path>, repository: Option<&Path>) -> Vec<PathBuf> {
    worktree
        .into_iter()
        .chain(repository)
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect()
}

/// Whether the canonical `file` is `plan` itself. The plan's folder is resolved but its file name
/// is not, so a symlink at the plan path that points elsewhere does not match.
fn is_own_plan(file: &Path, plan: &Path) -> bool {
    let (Some(folder), Some(name)) = (plan.parent(), plan.file_name()) else {
        return false;
    };
    is_local_path(plan)
        && std::fs::canonicalize(folder).is_ok_and(|folder| file == folder.join(name))
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

    fn no_plan() -> PathBuf {
        PathBuf::from("/nonexistent/plans/none.md")
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
        assert!(read(&run, "spec.md", &no_plan()).is_ok());
        fs::remove_dir(worktree.join(".git")).unwrap();
        fs::remove_dir(repository.join(".git")).unwrap();
        assert_eq!(
            read(&run, "spec.md", &no_plan()).err(),
            Some(DocumentError::NotAvailable {
                path: "spec.md".into()
            })
        );
    }

    #[test]
    fn a_document_over_a_megabyte_is_rejected_but_exactly_a_megabyte_is_served() {
        let (_dir, _, worktree, run) = setup("big.md");
        fs::write(worktree.join("big.md"), vec![b'a'; 1024 * 1024 + 1]).unwrap();
        assert_eq!(
            read(&run, "big.md", &no_plan()).err(),
            Some(DocumentError::TooLarge)
        );
        fs::write(worktree.join("big.md"), vec![b'a'; 1024 * 1024]).unwrap();
        assert!(read(&run, "big.md", &no_plan()).is_ok());
    }

    #[test]
    fn a_file_that_is_not_text_is_rejected() {
        let (_dir, _, worktree, run) = setup("bin.md");
        fs::write(worktree.join("bin.md"), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(
            read(&run, "bin.md", &no_plan()).err(),
            Some(DocumentError::NotText)
        );
    }

    /// Makes a FIFO with the `mkfifo` program (libc is not a dependency); `false` if unavailable.
    #[cfg(unix)]
    fn make_fifo(path: &Path) -> bool {
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Runs `work` on a thread and fails instead of hanging if it does not finish in 5 seconds.
    #[cfg(unix)]
    fn within_five_seconds<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(work());
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("blocked: a non-regular file was opened")
    }

    // Opening a FIFO blocks until a writer appears, so a FIFO named `x.md` must never be opened.
    #[cfg(unix)]
    #[test]
    fn a_fifo_named_like_a_document_is_not_available_and_does_not_block() {
        let (_dir, _, worktree, run) = setup("x.md");
        if !make_fifo(&worktree.join("x.md")) {
            eprintln!("skipped: mkfifo is not available");
            return;
        }
        let result = within_five_seconds(move || read(&run, "x.md", &no_plan()).err());
        assert_eq!(
            result,
            Some(DocumentError::NotAvailable {
                path: "x.md".into()
            })
        );
    }

    /// A plans folder with run 1's own plan and run 2's plan, and a run recording its own plan as spec.
    fn plans_setup() -> (TestDir, PathBuf, PathBuf, Run) {
        let (dir, repository, worktree, _) = setup("x.md");
        let plans = dir.0.path().join("data").join("plans");
        fs::create_dir_all(&plans).unwrap();
        fs::write(plans.join("1.md"), "# own plan").unwrap();
        fs::write(plans.join("2.md"), "# other plan").unwrap();
        let run = run(
            &repository,
            &worktree,
            &plans.join("1.md").to_string_lossy(),
        );
        (dir, plans, worktree, run)
    }

    #[test]
    fn the_runs_own_plan_is_served_even_when_the_worktree_is_gone() {
        let (_dir, plans, worktree, run) = plans_setup();
        fs::remove_dir_all(&worktree).unwrap();
        let plan = plans.join("1.md");
        let served = read(&run, &plan.to_string_lossy(), &plan).unwrap();
        assert!(served.html.contains("own plan"));
    }

    #[test]
    fn another_runs_plan_is_refused_even_when_recorded_as_spec() {
        let (_dir, plans, worktree, _) = plans_setup();
        let other = plans.join("2.md").to_string_lossy().into_owned();
        let repository = worktree.parent().unwrap().parent().unwrap().join("repo");
        let run = run(&repository, &worktree, &other);
        assert_eq!(
            read(&run, &other, &plans.join("1.md")).err(),
            Some(DocumentError::OutsideRoots)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_plan_path_pointing_elsewhere_is_refused() {
        let (dir, plans, _, run) = plans_setup();
        let secret = dir.0.path().join("secret.md");
        fs::write(&secret, "# secret").unwrap();
        fs::remove_file(plans.join("1.md")).unwrap();
        std::os::unix::fs::symlink(&secret, plans.join("1.md")).unwrap();
        let plan = plans.join("1.md");
        assert_eq!(
            read(&run, &plan.to_string_lossy(), &plan).err(),
            Some(DocumentError::OutsideRoots)
        );
    }

    #[test]
    fn a_recorded_markdown_file_inside_a_git_dir_is_refused() {
        let (_dir, repository, worktree, _) = setup("x.md");
        let in_repository = repository.join(".git/worktrees/x");
        fs::create_dir_all(&in_repository).unwrap();
        fs::write(in_repository.join("factory-plan.md"), "# git plan").unwrap();
        fs::write(worktree.join(".git/factory-plan.md"), "# git plan").unwrap();
        for reference in [
            in_repository.join("factory-plan.md"),
            worktree.join(".git/factory-plan.md"),
        ] {
            let reference = reference.to_string_lossy().into_owned();
            let run = run(&repository, &worktree, &reference);
            assert_eq!(
                read(&run, &reference, &no_plan()).err(),
                Some(DocumentError::OutsideRoots),
                "{reference}"
            );
        }
    }

    #[test]
    fn the_plans_folder_itself_is_refused() {
        let (_dir, plans, worktree, _) = plans_setup();
        let repository = worktree.parent().unwrap().parent().unwrap().join("repo");
        let folder = plans.to_string_lossy().into_owned();
        let run = run(&repository, &worktree, &folder);
        let error = read(&run, &folder, &plans.join("1.md")).err();
        assert!(error.is_some(), "{error:?}");
    }

    #[test]
    fn a_non_markdown_file_at_the_plan_location_is_refused() {
        let (_dir, plans, worktree, _) = plans_setup();
        let repository = worktree.parent().unwrap().parent().unwrap().join("repo");
        fs::write(plans.join("1.txt"), "text").unwrap();
        let reference = plans.join("1.txt").to_string_lossy().into_owned();
        let run = run(&repository, &worktree, &reference);
        assert_eq!(
            read(&run, &reference, &plans.join("1.txt")).err(),
            Some(DocumentError::NotMarkdown)
        );
    }

    #[test]
    fn an_oversized_plan_is_refused() {
        let (_dir, plans, _, run) = plans_setup();
        fs::write(plans.join("1.md"), vec![b'a'; 1024 * 1024 + 1]).unwrap();
        let plan = plans.join("1.md");
        assert_eq!(
            read(&run, &plan.to_string_lossy(), &plan).err(),
            Some(DocumentError::TooLarge)
        );
    }

    #[test]
    fn the_old_plan_beside_the_worktree_is_refused() {
        let (_dir, repository, worktree, _) = setup("x.md");
        let mut sibling = worktree.as_os_str().to_owned();
        sibling.push(".factory-plan.md");
        let sibling = PathBuf::from(sibling);
        fs::write(&sibling, "# old plan").unwrap();
        let run = run(&repository, &worktree, &sibling.to_string_lossy());
        assert_eq!(
            read(&run, &sibling.to_string_lossy(), &no_plan()).err(),
            Some(DocumentError::OutsideRoots)
        );
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
