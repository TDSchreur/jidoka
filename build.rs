use std::{env, fs, path::Path, process::Command};

fn files(root: &Path, dir: &Path, output: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read bundled skills") {
        let path = entry.expect("skill entry").path();
        if path.is_dir() {
            files(root, &path, output);
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let absolute = fs::canonicalize(&path).expect("skill path");
            output.push(format!(
                "({relative:?}, include_bytes!({:?}).as_slice()),",
                absolute.to_string_lossy()
            ));
        }
    }
}

/// Runs git and returns its trimmed output, or `None` when git or the repository is unavailable.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_string()).filter(|text| !text.is_empty())
}

/// Records what the binary was built from: official releases set `FACTORY_RELEASE=1`;
/// every other build is identified by its commit, when git can tell.
fn build_identity() {
    println!("cargo:rerun-if-env-changed=FACTORY_RELEASE");
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(branch_ref) = git(&["symbolic-ref", "-q", "HEAD"])
        .and_then(|branch| git(&["rev-parse", "--git-path", &branch]))
    {
        println!("cargo:rerun-if-changed={branch_ref}");
    }
    let commit = git(&["rev-parse", "--short=7", "HEAD"])
        .filter(|commit| commit.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_default();
    let release = env::var("FACTORY_RELEASE").is_ok_and(|value| value == "1");
    println!("cargo:rustc-env=FACTORY_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=FACTORY_BUILD_RELEASE={release}");
}

fn main() {
    build_identity();
    println!("cargo:rerun-if-changed=skills");
    println!("cargo:rerun-if-changed=.agents/skills");
    println!("cargo:rerun-if-changed=third-party");
    let mut output = Vec::new();
    for name in [
        "grill-with-docs",
        "grilling",
        "domain-modeling",
        "codebase-design",
        "to-spec",
        "to-tickets",
        "implement-spec",
        "implement-spec-fast",
        "implement-spec-balanced",
        "tdd",
        "code-review",
        "security-review",
        "github-actions-hardening",
        "pr",
    ] {
        files(
            Path::new("skills"),
            &Path::new("skills").join(name),
            &mut output,
        );
    }
    for name in [
        "factory",
        "factory-fast",
        "factory-balanced",
        "triage",
        "rust-best-practices",
    ] {
        files(
            Path::new(".agents/skills"),
            &Path::new(".agents/skills").join(name),
            &mut output,
        );
    }
    for name in [
        "mattpocock-skills-LICENSE",
        "awesome-copilot-LICENSE",
        "beautiful-rust-LICENSE",
    ] {
        let path = fs::canonicalize(Path::new("third-party").join(name)).unwrap();
        output.push(format!(
            "({:?}, include_bytes!({:?}).as_slice()),",
            format!("licenses/{name}"),
            path.to_string_lossy()
        ));
    }
    output.sort();
    let dest = Path::new(&env::var_os("OUT_DIR").unwrap()).join("bundled_skills.rs");
    fs::write(
        dest,
        format!(
            "static BUNDLED: &[(&str, &[u8])] = &[\n{}\n];",
            output.join("\n")
        ),
    )
    .unwrap();
}
