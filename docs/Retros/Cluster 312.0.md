# Cluster 312.0 retro — GitHub projector egress (arc closer)

> Tag **`v312.0.0`**. Phase XXIV (post-gate hardening). Git/GitHub projector, part 3 / finale. No new gate tag.

## What shipped

The egress half — a Maidan message in a linked thread now posts as a GitHub issue/PR comment —
completing the **bidirectional GitHub projector** (310 ingress → 311 links+inbound → 312 egress)
and, with it, the whole projector arc (both Slack + Git projectors):

- **`GithubSender` trait + `GithubApiClient`** (`github.rs`) — `post_comment(repo, issue, text)`
  via the GitHub REST API `POST /repos/{repo}/issues/{n}/comments` (bearer token, the required
  `User-Agent`, `Accept: application/vnd.github+json`). `GithubError`. A trait so tests inject a
  mock.
- **`route_message_to_github(state, thread_id, message)`** — no-op unless a `GithubSender` is
  configured; **skips GitHub-sourced messages** (the `metadata.github` tag from 311's ingress) so
  a projected inbound comment is never echoed back (loop prevention); resolves the thread's
  issue/PR via `get_github_issue_link_by_thread` and posts the comment. Best-effort, metered
  `maidan_github_egress_total{outcome}`.
- **Hooked into the notification-router `MessagePosted` arm** alongside the Slack egress call — no
  new bus consumer. `AppState.github_sender` + `attach_github_sender`; `main.rs` attaches a
  `GithubApiClient` when `MAIDAN_GITHUB_TOKEN` is set (ingress works without one).

## Surprises / decisions

- **Exact mirror of the Slack egress (309).** Same shape — sender trait + web client + a
  `route_message_to_*` hooked into the router, loop-safe via the origin metadata tag. The two
  projectors now sit side-by-side in the `MessagePosted` arm, each a cheap no-op without its
  sender.
- **Configured token, not the full GitHub App JWT flow.** Egress uses a bearer token
  (`MAIDAN_GITHUB_TOKEN` — a PAT or a pre-exchanged installation token). The GitHub App
  private-key → installation-token JWT auto-exchange (and Check Runs) are logged follow-ups; a
  configured token gets a working bidirectional projector without that machinery.
- **Loop prevention complete both ways** — ingress skips `Bot` comments (311), egress skips
  `metadata.github` messages (here). The e2e proves relay + skip-github-sourced + skip-unlinked.

## Capability table extension

GitHub egress: a Maidan message in a linked thread → a GitHub issue/PR comment; loop-safe both
ways. **Completes the bidirectional GitHub projector (310–312)** and the projector arc (Slack
307–309 + Git 310–312). No new capability/route.

## Risks identified + still open

- Best-effort egress (a failed comment post is logged + metered, not retried) — a durable egress
  outbox is a possible follow-up (low-harm: a dropped relay comment, unlike a lost notification
  email which has the mail outbox).
- GitHub App JWT/installation-token auto-exchange + Check Runs + non-`issue_comment` events are
  logged follow-ups; link management is store-level (a management surface can follow).

## Forward look

**Both projectors are complete** (Slack 307–309 + Git 310–312), config-gated + loop-safe. Per the
"both projectors, hold launch" plan, the next item — the **public launch** (arc #5) — is
**gated on the maintainer's explicit go** and is NOT auto-triggered. The projector arc concludes
here.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Completes the GitHub projector opened at
[[Retros/Cluster 310.0]]; mirrors the Slack egress ([[Retros/Cluster 309.0]]). Closes the
projector arc under the "both projectors, config-gated, hold launch" plan.
