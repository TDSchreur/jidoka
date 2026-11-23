"""Select CI for the changes in GitHub's PR test merge commit."""

import subprocess


ROOT_DOCS = (
    b"CODE_OF_CONDUCT.md",
    b"CONTRIBUTING.md",
    b"GLOSSARY.md",
    b"README.md",
    b"SECURITY.md",
    b"SUPPORT.md",
)

# Keep these directory prefixes in sync with the bundled skill lists in build.rs.
BUNDLED_SKILL_PREFIXES = (
    b"skills/jidoka-grill-with-docs/",
    b"skills/jidoka-grilling/",
    b"skills/jidoka-domain-modeling/",
    b"skills/jidoka-codebase-design/",
    b"skills/jidoka-improve-architecture/",
    b"skills/jidoka-to-spec/",
    b"skills/jidoka-to-tickets/",
    b"skills/jidoka-implement/",
    b"skills/jidoka-implement-fast/",
    b"skills/jidoka-implement-balanced/",
    b"skills/jidoka-tdd/",
    b"skills/jidoka-code-review/",
    b"skills/jidoka-security-review/",
    b"skills/jidoka-pr/",
    b"skills/jidoka-run-report/",
    b"skills/jidoka-setup/",
    b"skills/jidoka-awesome-copilot/",
    b"skills/jidoka-setup-knowledge-repo/",
    b"skills/jidoka-cross-repo-change/",
    b".agents/skills/jidoka/",
    b".agents/skills/jidoka-full/",
    b".agents/skills/jidoka-fast/",
    b".agents/skills/jidoka-balanced/",
    b".agents/skills/jidoka-procedures/",
    b".agents/skills/jidoka-triage/",
)


def main():
    subprocess.run(["git", "rev-parse", "--verify", "HEAD^2"], check=True, stdout=subprocess.DEVNULL)
    paths = subprocess.check_output(
        ["git", "diff", "--name-only", "-z", "--no-renames", "HEAD^1", "HEAD", "--"]
    ).split(b"\0")[:-1]

    if not paths:
        print("full")
        return

    mode = "docs"
    for path in paths:
        if not path.endswith(b".md"):
            mode = "full"
            break
        if path in ROOT_DOCS or path.startswith((b"docs/", b"changelog.d/")):
            continue
        if path.startswith(BUNDLED_SKILL_PREFIXES) or path.startswith(b"stages/"):
            mode = "content"
            continue
        mode = "full"
        break

    print(mode)


if __name__ == "__main__":
    main()
