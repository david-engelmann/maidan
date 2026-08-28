# Cluster 311.0 retro — GitHub projector: issue links + inbound routing

> Tag **`v311.0.0`**. Phase XXIV (post-gate hardening). Git/GitHub projector, part 2. No new gate tag.

## What shipped

The inbound half of the GitHub projector: a GitHub issue/PR maps to a Maidan thread, and comments
posted on GitHub appear in Maidan:

- **`maidan_github_issue_links` table** (pg 0052 / sqlite 0051; PK `(repo, issue_number)`) +
  `GithubIssueLink`/`NewGithubIssueLink` — maps a GitHub issue/PR to the Maidan
  channel/thread/member it projects into. Store (both backends): `link_github_issue` (upsert),
  `get_github_issue_link(repo, number)`, `get_github_issue_link_by_thread` (egress reverse lookup,
  312), `list_github_issue_links`, `unlink_github_issue`.
- **Inbound routing** (`github.rs::route_github_issue_comment`): an `issue_comment` event with
  `action == "created"` on a linked issue/PR posts `"{login}: {body}"` into the mapped thread (via
  `post_message_with_event` + `publish_stored`). Skips `comment.user.type == "Bot"` (our own egress
  echo) and stamps `metadata.github` for egress loop-prevention (312).

## Surprises / decisions

- **Composite key `(repo, issue_number)`.** A GitHub issue/PR is identified by repo full-name +
  number, so the link's PK is the pair — the same repo can have many linked issues (each its own
  Maidan thread). The e2e's second link (`o/r#43`) proves distinct issues are distinct links.
- **Loop prevention mirrors Slack's.** Ingress skips `Bot` comments (our egress identity) +
  stamps `metadata.github`; egress (312) will skip `metadata.github` messages. Same two-guard
  scheme as the Slack projector (307–309), just keyed on GitHub's payload shape.
- **Payload extraction is defensive.** `repository.full_name` / `issue.number` / `comment.body` are
  pulled with `and_then` chains — a malformed or partial payload simply doesn't route (best-effort;
  the ingress always ACKs so GitHub doesn't retry-storm).

## Capability table extension

`maidan_github_issue_links` + store (link/get/by-thread/list/unlink); GitHub `issue_comment`
events on a linked issue now post into the mapped Maidan thread. Link management is store-level (a
management surface can follow).

## Risks identified + still open

- **Only `issue_comment`** is routed (the core case). Other events (`issues`, `pull_request`,
  reviews) are ACKed but not projected — extendable later.
- **Egress is 312** — Maidan messages don't yet post back to GitHub; that needs a token
  (`MAIDAN_GITHUB_TOKEN`) and the `POST /repos/{repo}/issues/{n}/comments` call. The
  `metadata.github` tag + `Bot` skip are the loop-prevention groundwork.

## Forward look

**312** — egress: a Maidan message in a linked thread → a GitHub issue/PR comment (via the
configured token), skipping GitHub-sourced messages. Completes the Git projector; then hold at
the launch gate.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 310.0]] (the
ingress foundation); mirrors the Slack projector's link+inbound cluster ([[Retros/Cluster 308.0]]).
