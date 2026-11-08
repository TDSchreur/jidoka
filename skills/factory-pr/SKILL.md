---
name: factory-pr
description: "Write a pull request description that reviewers can scan quickly: a visual summary (diagram, diff-sketch or tree), before-and-after evidence, and merge danger (one-way or two-way door, blast radius). Use whenever drafting or rewriting a pull request or merge request description."
metadata:
  credits:
    skill: show-me
    author: Dex Horthy
    organisation: Humanlayer
    url: "https://github.com/humanlayer/skills/blob/main/plugins/show-me/skills/show-me/SKILL.md"
---

Draft the PR body from the template below. Return only the final description.

Use this template for writing the PR body:

```markdown
## Summary

<optional: linked tickets or spec>

<diagram, diff-sketch, or tree>

## Evidence

- **Before:** <screenshot/output/failing test run>
  **After:** <screenshot/output/passing test run>
- **Skill check:** <what was run and its outcome; only when the change edits agent skills>

## Merge Danger

**Door:** <one-way or two-way>

<optional: description>

**Blast Radius:** <one-word description>

<optional: potential ramifications of merge>

## Remaining review findings

<optional: one bullet per unresolved finding, with file and line>
```

## Title

When you also write the title, make it imperative and under about 70 characters, and describe the change rather than the ticket.

## Sections

Skip all preambles and keep prose brief. Use the user's domain language from `GLOSSARY.md`. Leave out an optional part when it has nothing to say.

### Summary

When the change has tickets or a spec, put them on one line above the visual, in the form the tracker links or closes them by (a closing keyword such as `Closes #123`, or a plain link).

Pick the smallest view that makes the key point clear.

- Pseudocode: logic or an algorithm.
- Call tree: runtime control flow.
- Component tree: UI structure, including state and module boundaries that matter.
- Shallow file tree: file responsibility or a broad refactor.
- Mermaid: component interaction, control flow, or data flow.
- `diff`: what changes when the surrounding shape already exists; match the diff shape to the topic (component, file layout, call tree, state or control flow).
- Whole block: most of it is new, omitted context would hide ownership or order, or the user needs a copyable target shape.

Examples of each are in [references/visuals.md](references/visuals.md).

#### Guidance

Place each visual next to the short text it supports. Keep only the calls, files, props, states, and boundaries a reviewer needs to understand the change.

You may use one of these, you may use several, it is unlikely you will use all of them. Use your judgement and don't overwhelm the reviewer.

### Evidence

Concrete evidence that the change works. Show a before and after.

Screenshots are the best evidence - when the environment is set up for it and the change is visual.

Execution-based evidence is the next best. Test results, console output. Show the exact test that now fails and passes, using pseudocode.

When the change edits agent skills, include a `**Skill check:**` line taken from the commits' `Skill check:` lines. Leave it out otherwise.

### Merge Danger

Describe whether it's a one-way or two-way door. You can walk back through two-way doors, but not one-way doors. A PR that is cheap to roll back is lower risk. Changes that involve destructive actions or hard-to-reverse decisions are one-way doors.

The blast radius is the potential impact or scope of the changes introduced by this PR. Consider all possibilities. Examples are layout shift, breakages for consumers, mobile responsiveness, etc.

### Remaining review findings

Include this section only when the work finished with review findings still unresolved, for example because the user chose to finish with them listed. Give one bullet per finding with its file and line, and keep the finding's wording. Leave the section out otherwise.
