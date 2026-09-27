# Issue tracker: GitHub

Issues and specs for this repo live as GitHub issues on `csnook/hab-bot`.

In Claude Code cloud sessions there is no `gh` CLI: use the GitHub MCP tools (`mcp__github__*`, loaded through ToolSearch). Where `gh` is available, its equivalents work the same way.

## Conventions

- **Create an issue**: `issue_write` with `method: create` (`gh issue create --title "..." --body "..."`).
- **Read an issue**: `issue_read` with `method: get`, plus `get_comments` and `get_labels` (`gh issue view <number> --comments`).
- **List issues**: `list_issues` with `labels` and `state` filters (`gh issue list --label ... --state ...`).
- **Comment on an issue**: `add_issue_comment` (`gh issue comment <number> --body "..."`).
- **Apply / remove labels**: `issue_write` with `method: update` and the full `labels` list (`gh issue edit <number> --add-label ... / --remove-label ...`).
- **Close**: comment first, then `issue_write` with `method: update`, `state: closed` and a `state_reason` (`gh issue close <number> --comment "..."`).

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Read the issue with its comments.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Destination / Notes / Decisions-so-far / Not-yet-specified / Out-of-scope body.
- **Child ticket**: a GitHub sub-issue of the map, created with `issue_write` (`method: create`, `parent_issue_number: <map>`). Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: a `Blocked by: #<n>, #<n>` line at the top of the child body. This is the canonical record: the MCP tools cannot create GitHub's native issue dependencies, so don't add native links from sessions that can, or the two will drift. A ticket is unblocked when every issue on its `Blocked by` line is closed.
- **Frontier query**: `issue_read` with `method: get_sub_issues` on the map; keep the open ones with no assignee and no open issue on their `Blocked by` line; first in sub-issue order wins.
- **Claim**: `issue_write` with `method: update` and `assignees: ["<driving dev's login>"]` (`gh issue edit <n> --add-assignee @me`), the session's first write.
- **Resolve**: `add_issue_comment` with the answer, close the issue with `state_reason: completed`, then append a context pointer (gist + link) to the map's Decisions-so-far.
