# Target layout

All paths are relative to the root of the project. Write files with `/` separators and LF line endings unless the project's `.gitattributes` or `.editorconfig` says otherwise.

## Where each agent looks

| Content | Codex | Claude Code | GitHub Copilot |
| --- | --- | --- | --- |
| Skills | `.agents/skills/` | `.claude/skills/` | `.agents/skills/` or `.claude/skills/` (also `.github/skills/`) |
| Scoped instructions | `AGENTS.md` (no glob scoping) | `.claude/rules/*.md` with `paths:` | `.github/instructions/*.instructions.md` with `applyTo:` |

- Codex scans `.agents/skills/` in every directory from the working directory up to the repository root. It reads `AGENTS.md` from the repository root down to the working directory and has no glob-scoped rules.
- Claude Code reads project skills only from `.claude/skills/`. It reads `.claude/rules/` (a rule with `paths:` loads when Claude reads or edits a matching file) and reads `AGENTS.md` only when the project has no `CLAUDE.md`.
- Copilot CLI and the Copilot cloud agent read project skills from `.github/skills/`, `.claude/skills/` and `.agents/skills/`. VS Code reads `.github/instructions/` and forwards skills from `.claude/skills/` and `.agents/skills/` to its agents.

So skills go to `.agents/skills/<name>/` and `.claude/skills/<name>/`: those two folders cover all three agents. Do not also write `.github/skills/`; Copilot would list the skill twice more. If the project already keeps its own skills in `.github/skills/` and the user wants the new ones there too, ask before adding a third copy and record it in the manifest.

Copy files rather than creating symbolic links: links need extra rights on Windows and are not followed the same way by every agent.

## Skills

Copy the upstream folder `skills/<name>/` to both:

- `.agents/skills/<name>/`
- `.claude/skills/<name>/`

Keep the folder name equal to the frontmatter `name`, and keep every file, including the skill's own `LICENSE` or `LICENSE.txt`. When the folder has no license file, add the upstream MIT license as `LICENSE`. Apply approved content edits to `SKILL.md` and its files identically in both copies.

## Instructions

Use the upstream file name without `.instructions.md` as `<name>`. Write the same adjusted body to two files, and add a Codex entry.

Copilot, `.github/instructions/<name>.instructions.md`: keep the upstream frontmatter, replace `applyTo` with the adjusted patterns, and add the attribution comment after the frontmatter:

```markdown
---
description: 'Guidelines for building C# applications'
applyTo: 'src/Api/**/*.cs, tests/Api.Tests/**/*.cs'
---

<!-- From github/awesome-copilot instructions/csharp.instructions.md at <sha>. MIT License, see .agents/awesome-copilot-LICENSE. Managed by .agents/awesome-copilot.json. -->

# C# Development
...
```

Claude Code, `.claude/rules/<name>.md`: the frontmatter holds only `paths` (Claude ignores other keys), followed by the same comment and body:

```markdown
---
paths:
  - "src/Api/**/*.cs"
  - "tests/Api.Tests/**/*.cs"
---

<!-- From github/awesome-copilot instructions/csharp.instructions.md at <sha>. MIT License, see .agents/awesome-copilot-LICENSE. Managed by .agents/awesome-copilot.json. -->

# C# Development
...
```

Leave out `paths` only for an instruction the user approved for every file; Claude then loads it in every session.

Codex: an entry in the managed section of the root `AGENTS.md` that names the patterns and links to the Copilot file, so Codex reads the full text when it works on matching files. The project root is the right place: Codex loads `AGENTS.md` files only from the root down to the working directory, never from a subdirectory it edits later, and every nested file also counts toward its instruction size limit (32 KiB by default). Copying instruction bodies into nested `AGENTS.md` files would therefore miss sessions started at the root, scope by folder instead of by file pattern, and also reach Claude and Copilot, which read `AGENTS.md` too. A short list of pointers avoids all three.

The section, with its markers:

```markdown
<!-- BEGIN awesome-copilot: managed by the jidoka-awesome-copilot skill and .agents/awesome-copilot.json -->
## Scoped instructions from awesome-copilot

Before you create or edit a file that matches one of these patterns, read the linked instructions and follow them.

- `src/Api/**/*.cs`, `tests/Api.Tests/**/*.cs`: [C# development](.github/instructions/csharp.instructions.md)
- `web/e2e/**/*.ts`: [Playwright TypeScript](.github/instructions/playwright-typescript.instructions.md)
<!-- END awesome-copilot -->
```

Append the section at the end of `AGENTS.md` the first time, and later replace only the text between the markers. If the user already moved the markers, respect the new position. If the project has an equivalent hand-written list (for example a "Scoped guides" section), ask whether to add the entries there instead; then record that in the manifest and edit only the lines you added.

## License attribution

The MIT license requires the copyright and permission notice in all copies or substantial portions. Write the upstream `LICENSE` file unchanged to `.agents/awesome-copilot-LICENSE`, keep or add a `LICENSE` in every skill folder as described above, and keep the attribution comment in every instruction file. A skill with its own non-MIT license keeps that license file; mention the license to the user (Apache-2.0, for example, also asks to keep any `NOTICE` file).

## Manifest

`.agents/awesome-copilot.json` records what this skill installed. It lives under `.agents/` because that folder is the agent-neutral home of the installed skills, it sits outside every agent's skill and instruction discovery path (Codex only scans `.agents/skills/`), and keeping it in one place lets one run manage all three agents. Commit it with the installed files.

```json
{
  "version": 1,
  "source": {
    "repository": "github/awesome-copilot",
    "commit": "8ecd5b17f02fe5fbebf35eccd1867265eded811f",
    "license": "MIT",
    "licenseFile": ".agents/awesome-copilot-LICENSE"
  },
  "agentsMdSection": "AGENTS.md",
  "items": [
    {
      "kind": "instruction",
      "name": "csharp",
      "upstreamPath": "instructions/csharp.instructions.md",
      "upstreamBlob": "<blob sha of the instruction file, or tree sha of the skill folder>",
      "upstreamApplyTo": ["**/*.cs"],
      "globs": ["src/Api/**/*.cs", "tests/Api.Tests/**/*.cs"],
      "globReason": "C# exists only in the API and its tests",
      "edits": [],
      "files": {
        ".github/instructions/csharp.instructions.md": "<blob sha as written>",
        ".claude/rules/csharp.md": "<blob sha as written>"
      }
    },
    {
      "kind": "skill",
      "name": "example-skill",
      "upstreamPath": "skills/example-skill",
      "license": "MIT",
      "edits": ["SKILL.md: replaced npm commands with pnpm"],
      "files": {
        ".agents/skills/example-skill/SKILL.md": "<blob sha as written>",
        ".agents/skills/example-skill/LICENSE": "<blob sha as written>",
        ".claude/skills/example-skill/SKILL.md": "<blob sha as written>",
        ".claude/skills/example-skill/LICENSE": "<blob sha as written>"
      }
    }
  ]
}
```

- `files` lists every file written, with its git blob hash from `git hash-object <path>` (the same command on Linux and Windows). Later runs use the hashes to tell an untouched file from one the user edited.
- `upstreamBlob` comes from `git -C <clone> ls-tree <commit> <upstreamPath>`: a blob sha for an instruction file, and a tree sha for a skill folder (the folder path without a trailing slash). The tree sha changes whenever any file in the skill changes, so it is the right thing to compare. Comparing them across commits shows which items changed upstream.
- `edits` holds one line per approved content change; an empty list means the body is upstream's.
- `agentsMdSection` names the file that holds the managed section, so removal finds it.

Write the JSON with two-space indentation and the items sorted by `kind` and `name`, so diffs stay small.
