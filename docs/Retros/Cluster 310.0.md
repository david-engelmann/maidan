# Cluster 310.0 retro — GitHub projector ingress foundation

> Tag **`v310.0.0`**. Phase XXIV (post-gate hardening). Git/GitHub projector, part 1. No new gate tag.

## What shipped

The ingress foundation for the Git projector (Expansion Bets, Bet 6) — a *projector*, not a bot
(no LLM in Maidan). Config-gated + inert until credentials are provided:

- **`github.rs`** — `GithubConfig::from_env` (`MAIDAN_GITHUB_WEBHOOK_SECRET` +optional
  `MAIDAN_GITHUB_TOKEN`; `None`/disabled when the secret is unset) and the `github_events`
  handler.
- **`POST /integrations/github/events`** — **unauthed** (GitHub authenticates via its
  `X-Hub-Signature-256`, verified in-handler), on the public router next to the Slack ingress.
  Returns `404` when unconfigured, `401` on a bad signature, `200` for the `ping` setup event,
  and ACKs other events (`issue_comment` → thread routing is 311).
- **`AppState.github` + `attach_github`**; wired in `main.rs` from `GithubConfig::from_env`.

## Surprises / decisions

- **Reused `webhooks::verify_signature`.** GitHub signs `sha256=hex(HMAC-SHA256(secret, body))`
  — byte-identical to Maidan's *own* outbound webhook signature format
  (`webhooks::sign_payload`). So GitHub signature verification is literally
  `crate::webhooks::verify_signature(secret, body, header)` — no new crypto, and the e2e signs
  with `webhooks::sign_payload`. (Unlike Slack's `v0:{ts}:{body}` scheme, which needed its own
  verifier + a replay window.)
- **No replay window.** GitHub's webhook signature has no timestamp component (unlike Slack), so
  verification is a pure HMAC match — nothing to bound.
- **`ping` is just a 200.** GitHub's setup handshake wants any 200 on the `ping` event (the
  event type is in the `X-GitHub-Event` header) — no challenge echo (unlike Slack's
  `url_verification`).
- **Config-gated to `404`.** Unconfigured deployments are byte-unchanged; no live GitHub App is
  needed to ship or test this.

## Capability table extension

New config-gated GitHub ingress (`POST /integrations/github/events`): `X-Hub-Signature-256`
verification + the `ping` handshake. No LLM, no live wiring yet.

## Risks identified + still open

- Events are ACKed but not yet routed — **311** adds the repo/issue link store + `issue_comment`
  → Maidan thread. **312** is egress (Maidan → issue/PR comment). Egress auth (a GitHub App
  installation-token JWT exchange) is the notable remaining complexity; a configured token (PAT
  or installation token) is the first cut, with the auto-JWT flow a follow-up.

## Forward look

**311** — GitHub repo/issue link mapping (`(repo, issue_number)` → Maidan thread) + inbound
`issue_comment` → thread. **312** — egress (a Maidan message in a linked thread → a GitHub
issue/PR comment). Then the projector arc is done and we hold at the launch gate.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens the Git projector (Bet 6) after
the Slack projector ([[Retros/Cluster 309.0]]); config-gated under the "both projectors, hold
launch" plan.
