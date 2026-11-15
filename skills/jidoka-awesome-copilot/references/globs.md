# Adjusting instruction globs

Upstream `applyTo` patterns assume a generic project: `**/*.cs` means "every C# file, wherever it is". In a real project that is often too broad (it also matches test fixtures, generated code or a sample folder) or simply wrong (`**/*.tsx` in a project whose React code lives in `web/`, next to a Storybook folder that should follow other rules). Rewrite each pattern so it matches the files the instruction is meant for, and nothing else.

## Steps

1. **Parse** the upstream `applyTo`: strip the quotes, split on commas, trim spaces. Treat `'*'` like `'**'` (every file). A missing `applyTo` means the instruction has no scope; propose one from its content.
2. **Expand braces** yourself before asking git: `**/*.{ts,tsx}` becomes `**/*.ts` and `**/*.tsx`. Git pathspecs do not support braces.
3. **List the matches** for each expanded pattern from the repository root:

   ```bash
   git ls-files --cached --others --exclude-standard -- ':(glob)**/*.cs'
   ```

   Count the matches per top-level directory (or per project root in a monorepo). The command is the same in PowerShell.
4. **Remove noise**: matches in generated, vendored, fixture, sample, snapshot or evaluation folders are not code the instruction should steer (EF Core `Migrations/`, `*.Designer.cs`, `*.g.cs`, `generated/`, `__snapshots__/`). Neither `applyTo` nor `paths` offers a dependable exclusion pattern, so "globs cannot exclude" is not a reason to keep them. Instead, list the sibling folders that hold real code:

   - Find the parent of the noise folder and list its other entries: `git ls-files -- ':(glob)AanvragenApp.DAL/**/*.cs'` shows `Entities/`, `Configuration/`, `Migrations/` and loose files.
   - Write one pattern for the real folders and one for loose files at that level: `AanvragenApp.DAL/{Entities,Configuration}/**/*.cs`, `AanvragenApp.DAL/*.cs`. Keep the broad pattern for the other roots: `AanvragenApp.{BLL,WebApi,Shared,Mail,AppHost,ServiceDefaults}/**/*.cs`, or `AanvragenApp.*/**/*.cs` when the noise sits in one project and you list that project separately.
   - Do this when the noise is a noticeable share of the matches (roughly a tenth or more) or is large per file, as migrations are. A stray generated file or two is not worth the longer pattern.
   - Tell the user the trade-off: a folder added later under that parent is not covered until the next re-sync. Record the enumerated folders in the manifest's `globReason` so update mode can spot new siblings and offer to add them.
   - When the noise cannot be separated by folder (generated and hand-written files side by side), keep the broader pattern and say which generated files it also matches.
5. **Narrow** to the directories that hold the real matches:
   - All matches under one or a few roots: replace the leading `**/` with those roots, for example `src/Api/**/*.cs`.
   - Matches spread over many unrelated roots, with no noise: keep `**/*.cs`. A shorter, equivalent pattern is better than a long list.
   - A pattern that matches nothing: drop it and say so. When no pattern of an instruction matches anything, the instruction is not relevant; leave it off the shortlist unless the user is about to add that code.
   - Upstream `**` (every file): propose the narrowest scope the content supports, for example `web/**` for a web accessibility guide or `infra/**` for an IaC guide. Keep `**` only for content that really applies everywhere, and point out that it loads into every Claude session.
6. **Split by role** when the content is about one role of files: a testing instruction gets the test locations (`tests/**/*.cs`, `**/*.test.ts`), not every source file.
7. **Check the result**: list the matches for the new patterns again and confirm they are the files you meant. Use paths relative to the repository root, with `/` separators, on every platform.

## Writing the patterns per agent

Write the same pattern list in each format:

- Copilot `applyTo`: one quoted, comma-separated string, for example `applyTo: 'src/Api/**/*.cs, tests/Api/**/*.cs'`. Brace expansion is allowed but keep it simple.
- Claude Code `paths`: a YAML list of quoted strings, one pattern per item. Brace expansion such as `src/**/*.{ts,tsx}` is supported. Leave `paths` out only when the instruction should load in every session (the `**` case).
- Codex: the patterns in backticks in the `AGENTS.md` entry, separated by commas.

## Examples

| Upstream | Project | Adjusted | Reason |
| --- | --- | --- | --- |
| `**/*.cs` | C# only in `src/Api/` and `tests/Api.Tests/` | `src/Api/**/*.cs`, `tests/Api.Tests/**/*.cs` | No C# elsewhere; keeps the rule out of `tools/` scripts added later in other languages |
| `**/*.tsx, **/*.ts, **/*.jsx, **/*.js, **/*.css` | React app in `web/`, Node scripts in `scripts/` | `web/src/**/*.{ts,tsx,css}` | No JSX; `scripts/` is not React code |
| `**/*.rs` | Rust in `src/`, `tests/`, `build.rs`, plus sample crates under `.agents/skills/*/evals/` | `src/**/*.rs`, `tests/**/*.rs`, `build.rs` | The eval fixtures are test data, not project code |
| `**` (accessibility) | Web UI in `web/`, API in `src/` | `web/**/*.{tsx,css,html}` | Accessibility rules only matter for UI files |
| `**/*.cs` | Projects `App.Api/`, `App.Bll/`, `App.Dal/` with 40 generated files in `App.Dal/Migrations/` | `App.{Api,Bll}/**/*.cs`, `App.Dal/{Entities,Configuration}/**/*.cs`, `App.Dal/*.cs` | Leaves out the generated migrations; a new folder under `App.Dal/` needs a re-sync |
| `**/*.tf` | No Terraform files | not proposed | Nothing matches |

Record each change in the manifest (`upstreamApplyTo`, `globs`, `globReason`) and show it to the user on the shortlist.
