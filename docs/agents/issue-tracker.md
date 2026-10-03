# Issue tracker: GitHub

Issues and specs for this repo live as GitHub issues on `csnook/hab-bot`.

Prefer the GitHub MCP tools (`mcp__github__*`, loaded through ToolSearch). They can't create GitHub's native issue dependencies, so use `gh api` for those: in Claude Code cloud sessions `gh` is a built-in client that supports only `gh api`, and elsewhere the GitHub CLI works the same way.

## Conventions

- **Create an issue**: `issue_write` with `method: create` (`gh issue create --title "..." --body "..."`).
- **Read an issue**: `issue_read` with `method: get`, plus `get_comments` and `get_labels` (`gh issue view <number> --comments`).
- **List issues**: `list_issues` with `labels` and `state` filters (`gh issue list --label ... --state ...`).
- **Comment on an issue**: `add_issue_comment` (`gh issue comment <number> --body "..."`).
- **Apply / remove labels**: `issue_write` with `method: update` and the full `labels` list (`gh issue edit <number> --add-label ... / --remove-label ...`).
- **Close**: comment first, then `issue_write` with `method: update`, `state: closed` and a `state_reason` (`gh issue close <number> --comment "..."`).

## Blocking

Blocking uses GitHub's native issue dependencies, so the issue view shows what's ready, plus a written record in the ticket body.

- **Add a link**: `gh api --method POST repos/csnook/hab-bot/issues/<n>/dependencies/blocked_by -F issue_id=<id>`, where `<id>` is the blocker's database id (`gh api repos/csnook/hab-bot/issues/<blocker> --jq .id`), not its number.
- **Read**: an issue's `issue_dependencies_summary.blocked_by` counts its open blockers. A ticket is unblocked when that is 0.
- **Keep the body in step**: whenever a link is added or removed, update the ticket's written record too.

## Implementation tickets

Used by `/to-tickets`.

- **Parent**: each release's tickets are sub-issues of one parent issue, such as [First release](https://github.com/csnook/hab-bot/issues/35), created in dependency order, blockers first.
- **Body**: Parent, What to build, Acceptance criteria, and Blocked by, a list of `- #<n> <title>` or "None (can start immediately)".
- **Labels**: `ready-for-agent` or `ready-for-human` (see `triage-labels.md`).

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
- **Blocking**: a `Blocked by: #<n>, #<n>` line at the top of the child body, plus native links (see Blocking above). The map [Reminder app design](https://github.com/csnook/hab-bot/issues/1) predates native links and has the line only. A ticket is unblocked when every issue it's blocked by is closed.
- **Frontier query**: `issue_read` with `method: get_sub_issues` on the map; keep the open ones with no assignee and no open blockers (native links, or the `Blocked by` line on older tickets); first in sub-issue order wins.
- **Claim**: `issue_write` with `method: update` and `assignees: ["<driving dev's login>"]` (`gh issue edit <n> --add-assignee @me`), the session's first write.
- **Resolve**: `add_issue_comment` with the answer, close the issue with `state_reason: completed`, then append a context pointer (gist + link) to the map's Decisions-so-far.
