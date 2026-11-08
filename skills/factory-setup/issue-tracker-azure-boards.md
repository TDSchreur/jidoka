# Issue tracker: Azure Boards

Issues and specs for this repo live as work items in Azure Boards, whatever host the code lives on. Use the `az` CLI with the `azure-devops` extension for all operations.

## Configuration

Setup fills in these values from the project itself. Use them in every command below; never guess a type or state name, because they differ per process (Agile, Scrum, Basic, CMMI or a custom one). If a type or state you need is missing here, stop and ask the user to rerun setup instead of guessing.

- **Organization URL**: `<org-url>` (for example `https://dev.azure.com/<organization>`)
- **Project**: `<project>`

| Used for | Work item type   |
| -------- | ---------------- |
| Specs    | `<spec-type>`    |
| Tickets  | `<ticket-type>`  |
| Bugs     | `<bug-type>`     |

Each cell lists all states of that category for that type, comma-separated. The first one is the state to set; queries and blocker checks use all of them.

| Work item type  | Proposed | InProgress | Completed | Removed | Wontfix state |
| --------------- | -------- | ---------- | --------- | ------- | ------------- |
| `<spec-type>`   | `<...>`  | `<...>`    | `<...>`   | `<...>` | `<...>`       |
| `<ticket-type>` | `<...>`  | `<...>`    | `<...>`   | `<...>` | `<...>`       |
| `<bug-type>`    | `<...>`  | `<...>`    | `<...>`   | `<...>` | `<...>`       |

An item is **open** while its state is in the Proposed or InProgress category, and **closed** once it is in the Completed or Removed category.

## Authentication

Sign in once with `az login` (Microsoft Entra ID), or with a personal access token through `az devops login --org <org-url>` (it reads the token from the prompt or stdin) or the `AZURE_DEVOPS_EXT_PAT` environment variable. Never print, echo or log a token.

## Conventions

Every command passes `--org <org-url> --detect false`, so nothing depends on `az devops configure` defaults or the git remote. Commands that take a project get `--project <project>` (`-p`); `az devops invoke` gets it as the route parameter `project=<project>`. Work item ids are unique across the organization, so commands that take `--id` need no project.

- **Create a work item**: `az boards work-item create --org <org-url> --detect false --project <project> --type "<ticket-type>" --title "..." --description "..."`. Use `<spec-type>` for a spec and `<bug-type>` for a bug. `--description` is HTML, not Markdown: write headings, paragraphs and lists as HTML (`<h2>`, `<p>`, `<ul><li>`), or line breaks are lost. For a multi-line body, read it from a heredoc into a variable and pass `--description "$body"`. Add tags at creation with `--fields "System.Tags=<tag>; <tag>"`.
- **Read a work item**: `az boards work-item show --org <org-url> --detect false --id <id> --expand relations` returns the fields (`System.Title`, `System.Description`, `System.State`, `System.Tags`, `System.WorkItemType`, `System.AssignedTo`) and the links. Read the comments with `az devops invoke --org <org-url> --detect false --area wit --resource comments --route-parameters "project=<project>" "workItemId=<id>" --api-version 7.1-preview`. The `-preview` suffix is required; `7.1` alone is refused.
- **List work items**: run a WIQL query with `az boards query --org <org-url> --detect false --project <project> --wiql "..."`. For example, open tickets with a triage role: `SELECT [System.Id], [System.Title], [System.State], [System.Tags] FROM WorkItems WHERE [System.TeamProject] = '<project>' AND [System.WorkItemType] = '<ticket-type>' AND [System.Tags] CONTAINS '<ready-for-agent tag>' AND [System.State] NOT IN ('<all Completed and Removed states of the type>') ORDER BY [System.CreatedDate]`. WIQL has no state-category filter, so list every Completed and Removed state from the configuration. For items without any triage role, combine `[System.Tags] NOT CONTAINS '<tag>'` for each role tag (see **Apply / remove triage roles**).
- **Comment on a work item**: `az boards work-item update --org <org-url> --detect false --id <id> --discussion "..."`. The comment is HTML, like the description.
- **Apply / remove triage roles**: triage roles are tags in the `System.Tags` field, a list separated by `; `. The tag names come from `docs/agents/triage-labels.md`; when that file is absent, use the canonical role names: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. A tag is created the first time it is used.
  - To apply a role, run `az boards work-item update --org <org-url> --detect false --id <id> --fields "System.Tags=<tag>"`. Azure adds the tags you pass to the ones the item already has, so this call never removes a tag, not even with an empty value.
  - To remove a role, or move between roles:
    1. Read the current tags with `az boards work-item show --org <org-url> --detect false --id <id> --query 'fields."System.Tags"' -o tsv` (`--fields` cannot be combined with the expanded output `show` returns).
    2. Drop the old role and add the new one in the list.
    3. Write the whole list back with a JSON Patch `replace`, which `az boards` cannot send: `az rest --method patch --resource 499b84ac-1321-427f-aa17-267ca6975798 --headers Content-Type=application/json-patch+json --uri "<org-url>/<project>/_apis/wit/workitems/<id>?api-version=7.1" --body '[{"op":"replace","path":"/fields/System.Tags","value":"<tag>; <tag>"}]'`. Percent-encode the project in the URI (a space is `%20`). `499b84ac-1321-427f-aa17-267ca6975798` is the Azure DevOps resource that `az rest` asks the `az login` account a token for. When signed in only with a personal access token, send the same request with curl, passing the token on standard input so it never appears in the process list: `printf 'user = ":%s"\n' "$AZURE_DEVOPS_EXT_PAT" | curl -sS -K - -X PATCH -H "Content-Type: application/json-patch+json" --data '<the same body>' "<the same URI>"`. Never print, echo or log the token.
- **Change state**: `az boards work-item update --org <org-url> --detect false --id <id> --state "<state>"`, with a state from the configuration for that item's type.
- **Close**: post the explanation first with `--discussion`, then set the Completed state of the item's type with `--state` in a second `update` call.
- **Close as wontfix**: post the explanation first with `--discussion`, then set the Wontfix state of the item's type with `--state`, and set the configured `wontfix` tag as in **Apply / remove triage roles**.
- **Make a work item a child of a parent**: `az boards work-item relation add --org <org-url> --detect false --id <child> --relation-type parent --target-id <parent>`. A child shows the link as `System.LinkTypes.Hierarchy-Reverse` and the parent as `System.LinkTypes.Hierarchy-Forward` in **Read a work item**.
- **Blocking**: Azure Boards' Predecessor link, the canonical, UI-visible representation. To record that `<blocker>` blocks `<child>`: `az boards work-item relation add --org <org-url> --detect false --id <child> --relation-type Predecessor --target-id <blocker>` (the blocker sees it as a Successor link). `az boards work-item relation list-type --org <org-url> --detect false` lists the link names the organization accepts.
- **Count open blockers**: read the item with `--expand relations`, take every relation with `rel` equal to `System.LinkTypes.Dependency-Reverse` (Predecessor), and read each target id's `System.State`. An item is unblocked when every predecessor is in a Completed or Removed category state for its type.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Fixed for Azure Boards: pull requests are not work items, so `/factory-triage` does not read them from this tracker.)_

## When a skill says "publish to the issue tracker"

Create a work item as in **Create a work item**: `<spec-type>` for a spec, `<ticket-type>` for a ticket, `<bug-type>` for a bug.

## When a skill says "fetch the relevant ticket"

Read it as in **Read a work item** above, including its comments.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single parent work item with **child** work items as tickets.

- **Map**: a single `<spec-type>` work item tagged `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body in its description. Create it with `--fields "System.Tags=wayfinder:map"`.
- **Child ticket**: a `<ticket-type>` work item made a child of the map (see **Make a work item a child of a parent**), tagged `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: see **Blocking** under Conventions.
- **Frontier query**: read the map with `--expand relations` and take its children (relations with `rel` equal to `System.LinkTypes.Hierarchy-Forward`), in ascending work item id order, because Azure does not guarantee the order of relations. Keep the open ones without `System.AssignedTo`, drop any with an open predecessor (see **Count open blockers**); the lowest id wins.
- **Claim**: `az boards work-item update --org <org-url> --detect false --id <id> --assigned-to "<sign-in name of the driving dev>" --state "<InProgress state>"`, the session's first write.
- **Resolve**: post the answer with `--discussion`, then set the Completed state of the ticket's type, then append a context pointer (gist + link) to the map's Decisions-so-far by rewriting the map's description with `--description`.

## Unsupported or limited

- **Comments**: editing or deleting a comment is not covered.
- **Assign to me**: there is no `@me`. Pass the user's sign-in name or email to `--assigned-to`.
- **Closing reasons**: `--reason` values are process-specific and are not part of the configuration; leave them to the process default.
