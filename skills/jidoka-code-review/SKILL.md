---
name: jidoka-code-review
description: "Review the changes since a fixed point (commit, branch, tag, or merge-base) along two axes: Standards (does the code follow this repo's documented coding standards?) and Spec (does the code match what the originating issue/spec asked for?). Runs both reviews in parallel sub-agents and reports them side by side. Use when the user wants to review a branch, a PR, work-in-progress changes, or asks to \"review since X\". For a security review use jidoka-security-review."
---

Two-axis review of the diff between `HEAD` and a fixed point the user supplies:

- **Standards**: does the code conform to this repo's documented coding standards?
- **Spec**: does the code faithfully implement the originating issue / spec?

Both axes run as **parallel sub-agents** so they don't pollute each other's context, then this skill aggregates their findings.

A spec given as a file path needs no tracker. When the spec is a tracker issue that you must fetch and the tracker was not provided to you, tell the user to run `/jidoka-setup`.

## Process

### 1. Pin the fixed point

Whatever the user said is the fixed point (a commit SHA, branch name, tag, `main`, `HEAD~5`, etc.). If they didn't specify one, ask for it.

Capture the diff command once: `git diff <fixed-point>...HEAD` (three-dot, so the comparison is against the merge-base). Also note the list of commits via `git log <fixed-point>..HEAD --oneline`.

Before going further, confirm the fixed point resolves (`git rev-parse <fixed-point>`) and the diff is non-empty. A bad ref or empty diff should fail here, not inside two parallel sub-agents.

### 2. Identify the spec source

Look for the originating spec, in this order:

1. Issue references in the commit messages (`#123`, `Closes #45`, etc.), fetched via the workflow in the tracker doc.
2. A path the user passed as an argument.
3. A spec file under `docs/`, `specs/`, or `.scratch/` matching the branch name or feature.
4. If nothing is found, ask the user where the spec is. If they say there isn't one, the **Spec** sub-agent will skip and report "no spec available".

### 3. Identify the standards sources

Search the repo for every file that documents how code should be written. When `CODING_STANDARDS.md` or `CONTRIBUTING.md` exists, it must be on the list.

On top of whatever the repo documents, the Standards axis always carries the **smell baseline** below: a fixed set of Fowler code smells (_Refactoring_, ch.3) that applies even when a repo documents nothing. Two rules bind it:

- **The repo overrides.** A documented repo standard always wins; where it endorses something the baseline would flag, suppress the smell.
- **Always a judgement call.** Each smell is a labelled heuristic ("possible Feature Envy"), never a hard violation. Like any standard here, skip anything tooling already enforces.

Each smell reads *what it is* → *how to fix*; match it against the diff:

- **Mysterious Name**: a function, variable, or type whose name doesn't reveal what it does or holds. → rename it; if no honest name comes, the design's murky.
- **Duplicated Code**: the same logic shape appears in more than one hunk or file in the change. → extract the shared shape, call it from both.
- **Feature Envy**: a method that reaches into another object's data more than its own. → move the method onto the data it envies.
- **Data Clumps**: the same few fields or params keep travelling together (a type wanting to be born). → bundle them into one type, pass that.
- **Primitive Obsession**: a primitive or string standing in for a domain concept that deserves its own type. → give the concept its own small type.
- **Repeated Switches**: the same `switch`/`if`-cascade on the same type recurs across the change. → replace with polymorphism, or one map both sites share.
- **Shotgun Surgery**: one logical change forces scattered edits across many files in the diff. → gather what changes together into one module.
- **Divergent Change**: one file or module is edited for several unrelated reasons. → split so each module changes for one reason.
- **Speculative Generality**: abstraction, parameters, or hooks added for needs the spec doesn't have. → delete it; inline back until a real need shows.
- **Message Chains**: long `a.b().c().d()` navigation the caller shouldn't depend on. → hide the walk behind one method on the first object.
- **Middle Man**: a class or function that mostly just delegates onward. → cut it, call the real target direct.
- **Refused Bequest**: a subclass or implementer that ignores or overrides most of what it inherits. → drop the inheritance, use composition.

The Standards axis also carries a **cross-platform checklist**, with the same two rules, applied only when the project targets more than one platform (more than one operating system, or a platform-gated build). Match it against the diff:

- **Paths**: hard-coded separators or paths built by string concatenation instead of per component; absolute or home-relative paths assumed to exist.
- **Permissions**: file modes, executable bits, symlinks, or Unix-only APIs used without a guard or an equivalent on the other platforms.
- **Platform-gated code**: a `cfg`, `#ifdef`, or `os.name` branch where code, imports, or variables are used in one branch only (unused or missing on the other platform), or where one platform's branch is never compiled or run by the checks.
- **Line endings**: text compared, hashed, or written assuming `\n`, or files whose line endings the diff changes.
- **Case sensitivity**: file names that differ only by case, or lookups that assume a case-sensitive file system.
- **Shell and processes**: shell syntax, quoting, program names, `PATH` lookup, or signals that differ between shells and operating systems.

### 4. Spawn both sub-agents in parallel

Issue both sub-agent calls together, in the foreground, and aggregate the reports they return.

If the caller gives you a model and an effort for the review agents, start both sub-agents with them as "Starting a routed agent" in [model-routing.md](../jidoka-procedures/chunks/model-routing.md) of `jidoka-procedures` describes, and start fresh subagents, not forks, since a fork ignores `model`. That section also covers the check of the model each agent ran on; when it finds a different model, report that with the fallback line from "Reporting" in the same file. On a host it does not cover, use that host's own way of setting a subagent's model and effort. When the host does not take the model or refuses a setting, follow the bullet in "Starting a routed agent" that starts "If the recorded model is not one the host takes", and report it with the "model routing not applied: <reason>" line from "Reporting". Without a model and effort from the caller, start the sub-agents exactly as described here.

**Standards sub-agent prompt** should include:

- The full diff command and commit list.
- The list of standards-source files you found in step 3, **plus the smell baseline and the cross-platform checklist from step 3** pasted in full (the sub-agent has no other access to them).
- The agent-skill check, pasted in full: when the diff changes what an agent skill (a `SKILL.md` or a file it references) makes the agent do, decide, output or promise, and no commit carries a `Skill check:` line with eval or forward-test evidence, report it as a judgement call. Wording-only changes are exempt.
- The brief: "Report, per file/hunk where relevant, (a) every place the diff violates a documented standard: cite the standard (file + the rule); (b) any baseline smell you spot: name it and quote the hunk; and (c) any agent-skill change without a `Skill check:` as described above. Distinguish hard violations from judgement calls: documented-standard breaches can be hard, but baseline smells, cross-platform findings and the agent-skill check are always judgement calls, and a documented repo standard overrides the baseline. Skip anything tooling enforces. Under 400 words."

**Spec sub-agent prompt** should include:

- The diff command and commit list.
- The path or fetched contents of the spec.
- The brief: "Report: (a) requirements the spec asked for that are missing or partial; (b) behaviour in the diff that wasn't asked for (scope creep); (c) requirements that look implemented but where the implementation looks wrong. Quote the spec line for each finding. Under 400 words."

If the spec is missing, skip the Spec sub-agent and note this in the final report.

### 5. Aggregate

Present the two reports under `## Standards` and `## Spec` headings, verbatim or lightly cleaned. Do **not** merge or rerank findings, because the two axes are deliberately separate (see _Why two axes_).

End with a one-line summary: total findings per axis, and the worst issue _within each axis_ (if any). Don't pick a single winner across axes: that's the reranking the separation exists to prevent.

## Why two axes

A change can pass one axis and fail the other:

- Code that follows every standard but implements the wrong thing → **Standards pass, Spec fail.**
- Code that does exactly what the issue asked but breaks the project's conventions → **Spec pass, Standards fail.**

Reporting them separately stops one axis from masking the other.
