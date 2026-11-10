---
description: 'Guidance for authoring project-maintained agent skills'
applyTo: 'skills/**,.agents/skills/**'
---

# Skill authoring

- Give each skill a directory containing `SKILL.md` with YAML frontmatter: a lowercase hyphenated `name` matching the directory and a `description` that says when to use it.
- Keep the workflow focused and concise. Include instructions that change decisions, and avoid repeating general agent capabilities.
- Put supporting material in the skill directory and reference it from `SKILL.md`. Include files only when the skill needs them.
- Preserve user intent and existing permission boundaries. Make dependencies and expected outputs clear.
- Skills bundled by `jidoka install` (listed in `build.rs`) are used on any project. Do not name this repository's scripts, branches, tools or conventions in them; keep those in `AGENTS.md`, the README or `.github/instructions/`. A test in `src/install.rs` enforces this.
- Validate frontmatter and check that referenced files exist. Treat copied upstream skills as third-party code; preserve their attribution and license.

## Changing a skill

Scale the check to the change. The split is the same as for wording versus behaviour in the Reworks section of [jidoka-procedures](../../.agents/skills/jidoka-procedures/SKILL.md).

- **Behaviour change**: the change affects what the skill makes the agent do, decide, output or promise. Load your agent's `skill-creator` skill, then verify the change with realistic prompts run by a fresh agent that has the changed skill. On Claude Code, write 2-3 eval cases and run them with skill-creator's eval workflow (not the full benchmark loop). On Codex, run one independent forward-test as its `skill-creator` describes.
- **Wording-only change**: typos, clearer phrasing, formatting or links, with no effect on what the agent does. No `skill-creator` needed.
- Record the result as a `Skill check:` line in the commit message: what was run and its outcome, or `wording only`. When no `skill-creator` skill is available, say so in that line and still forward-test with a fresh agent.
