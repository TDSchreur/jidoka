"""Select CI for the changes in GitHub's PR test merge commit."""

import subprocess


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
        if path in (b"README.md", b"GLOSSARY.md") or path.startswith((b"docs/", b"changelog.d/")):
            continue
        if path.startswith((b"skills/", b".agents/skills/", b"stages/")):
            mode = "content"
            continue
        mode = "full"
        break

    print(mode)


if __name__ == "__main__":
    main()
