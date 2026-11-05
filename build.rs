use std::{env, fs, path::Path};

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

fn main() {
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
