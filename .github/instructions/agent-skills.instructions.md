---
description: 'Guidance for authoring project-maintained agent skills'
applyTo: 'skills/**/SKILL.md,.agents/skills/**/SKILL.md'
---

# Skill authoring

- Give each skill a directory containing `SKILL.md` with YAML frontmatter: a lowercase hyphenated `name` matching the directory and a `description` that says when to use it.
- Keep the workflow focused and concise. Include instructions that change decisions, and avoid repeating general agent capabilities.
- Put supporting material in the skill directory and reference it from `SKILL.md`. Include files only when the skill needs them.
- Preserve user intent and existing permission boundaries. Make dependencies and expected outputs clear.
- Skills bundled by `factory install` (listed in `build.rs`) are used on any project. Do not name this repository's scripts, branches, tools or conventions in them; keep those in `AGENTS.md`, the README or `.github/instructions/`. A test in `src/install.rs` enforces this.
- Validate frontmatter and check that referenced files exist. Treat copied upstream skills as third-party code; preserve their attribution and license.
