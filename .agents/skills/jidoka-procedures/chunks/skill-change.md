## Changing agent skills

In Build, routed or not, when a ticket changes what an agent skill (a `SKILL.md` or the files it references) makes the agent do, decide, output or promise, use a skill-creator skill if the agent has one, and verify the change with realistic prompts run by a fresh agent that has the changed skill. Follow the project's own skill-authoring guidance where it has any. Record a `Skill check:` line in the commit message: what was run and its outcome, and when no skill-creator skill is available, say so there. Changes to wording only (typos, phrasing, formatting, links) need no check; the line says `wording only`.

