---
name: jidoka-improve-architecture
description: Scan a codebase for deepening opportunities, report them as Markdown in the chat, grill through the one the user picks and file it as a spec. Use when the user asks to find refactoring or deepening opportunities, or about large or hard-to-change modules.
disable-model-invocation: true
---

# Improve Architecture

Surface architectural friction and propose **deepening opportunities**: refactors that turn shallow modules into deep ones. The aim is testability and AI-navigability. This skill finds and specifies; it does not refactor.

It is informed by the project's domain model and built on a shared design vocabulary:

- Load the `jidoka-codebase-design` skill for the architecture vocabulary (**module**, **interface**, **depth**, **seam**, **adapter**, **leverage**, **locality**) and its principles (the deletion test, "the interface is the test surface", "one adapter = hypothetical seam, two = real"). Use these terms exactly in every suggestion, and don't drift into "component," "service," "API," or "boundary."
- The domain language in `GLOSSARY.md` gives names to good seams; ADRs in `docs/adr/` record decisions this skill should not re-litigate.

## Process

### 1. Explore

**Scope before you scan: YAGNI.** Deepening a module pays off by making future changes to it easier, so put extra weight on the parts of the codebase that have recently changed. Decide *where* to look before you look:

- If the user named a direction (a module, a subsystem, a pain point), take it, and skip the inference below.
- Otherwise, walk back a good stretch of the commit history (`git log --oneline`) to find the codebase's hot spots, the files and areas that keep coming up, and let those paths pull your attention first. If the changes are scattered with no clear hot spot, widen the net.

Read the project's domain glossary (`GLOSSARY.md`) and any ADRs in the area you're touching first.

Then spawn a sub-agent with a fresh context window to walk the codebase. Don't follow rigid heuristics; explore organically and note where you experience friction:

- Where does understanding one concept require bouncing between many small modules?
- Where are modules **shallow**, with an interface nearly as complex as the implementation?
- Where have pure functions been extracted just for testability, but the real bugs hide in how they're called (no **locality**)?
- Where do tightly-coupled modules leak across their seams?
- Which parts of the codebase are untested, or hard to test through their current interface?

Size alone (a long file or function) is a prompt to look, not a reason to split. A candidate needs a second reason to change or a shallow interface.

Apply the **deletion test** to anything you suspect is shallow: would deleting it concentrate complexity, or just move it? A "yes, concentrates" is the signal you want.

### 2. Present candidates as a Markdown report

Write the report in the chat. Do not create a file. Use one entry per candidate:

```markdown
### 1. <Candidate name>
- **Files**: the files or modules involved
- **Problem**: why the current architecture causes friction
- **Solution**: plain English description of what would change
- **Benefits**: in terms of locality and leverage, and how tests would improve
- **Strength**: Strong / Worth exploring / Speculative
- **ADR conflict**: only when relevant
```

End with a **Top recommendation**: which candidate you'd tackle first and why. Keep the whole report concise.

**Use GLOSSARY.md vocabulary for the domain, and the `jidoka-codebase-design` vocabulary for the architecture.** If `GLOSSARY.md` defines "Order," talk about "the Order intake module," not "the FooBarHandler," and not "the Order service."

**ADR conflicts**: if a candidate contradicts an existing ADR, only surface it when the friction is real enough to warrant revisiting the ADR. Mark it in the entry (e.g. _"contradicts ADR-0007, but worth reopening because..."_). Don't list every theoretical refactor an ADR forbids.

Do NOT propose interfaces yet. After the report, ask the user: "Which of these would you like to explore?" Then stop and wait. Create no files and no issues before the user picks.

### 3. Grilling loop

Once the user picks a candidate, load the `jidoka-grilling` skill to walk the decision tree with them: constraints, dependencies, the shape of the deepened module, what sits behind the seam, what tests survive.

Side effects happen inline as decisions crystallize; load the `jidoka-domain-modeling` skill to keep the domain model current as you go:

- **Naming a deepened module after a concept not in `GLOSSARY.md`?** Add the term to `GLOSSARY.md`. Create the file lazily if it doesn't exist.
- **Sharpening a fuzzy term during the conversation?** Update `GLOSSARY.md` right there.
- **User rejects the candidate with a load-bearing reason?** Offer an ADR, framed as: _"Want me to record this as an ADR so future architecture reviews don't re-suggest it?"_ Only offer when the reason would actually be needed by a future explorer to avoid re-suggesting the same thing; skip ephemeral reasons ("not worth it right now") and self-evident ones.
- **Want to explore alternative interfaces for the deepened module?** Load `jidoka-codebase-design` and use its design-it-twice parallel sub-agent pattern.

### 4. File the spec

When the user confirms the deepened design, write it as a spec (use the `jidoka-to-spec` skill's template if available) and publish it to the project's configured issue tracker, as the docs from setup describe (for example `docs/agents/issue-tracker.md`). Mention the glossary terms and ADRs it relies on.

If no tracker is configured, give the spec in the chat and say how to set one up with `jidoka-setup`.

File only the chosen candidate; do not file the others. Do not start refactoring.
