# Cluster 315.0 retro — pre-launch correctness & DX + research-sweep fold

> Tag **`v315.0.0`**. Phase XXIV (post-gate hardening). First cluster of the 2026-08-28
> research-sweep program. No new gate tag.

## What shipped

The small, verified correctness/DX fixes from the pre-launch audit — plus the fold of the
whole 2026-08-28 research sweep into Open Work (the planning that defines 315–318 + the
fidelity/context flagship arc).

- **`hash-v1` boot warning** (`main.rs`) — the default embedding provider is a deterministic
  hash, not a real embedding, so "semantic search" silently returns near-random results if
  `MAIDAN_EMBEDDING_PROVIDER` is unset. A stranger who deploys without it had **no signal**.
  Now a loud `warn!` at boot names the problem and the fix (`openai-compatible`). Lexical
  search is unaffected.
- **README "Run it (SQLite, no Docker)" boot bug** — the block used a **28-byte**
  `MAIDAN_SESSION_SECRET`; `session/cookie.rs:18` requires ≥32, so the command errored before
  serving. Fixed to a ≥32-byte value (the 314 *headline* one-liner was fixed; this sibling was
  missed).
- **`event_stream` replay cursor-advance** (`event_stream.rs`) — the replay path discarded a
  failed `advance_delivery_cursor` with `let _ =`. Correctness was always safe (a stuck cursor
  re-delivers, never skips — at-least-once holds), but the failure was invisible; it now logs.
- **Defensive `ensure_acting_member`** on the legacy `/members/:id/mentions` + `/inbox`
  handlers (see the correction below).
- **Open Work fold** — corrected the backlog to `v314` (SDKs published, mail/Slack/projectors
  done, atomicity done, import done, DM/subscribe-grant risks closed) and added the sequenced
  315–318 + fidelity/context flagship arc with a locked anti-goals block.

## The load-bearing correction: the "live authz defect" was a false positive

The sweep's **headline 315 item** was "legacy `/members/:id/mentions` + `/inbox` are not
self-only — a session can read/advance another member's inbox." **On verification it is a
false positive on reachability.** These handlers are mounted **only** on the bearer-only
`protected` router (`auth::middleware`, which rejects session cookies → `401`); there is **no
`/ui/api` session mount** for them (unlike the notification handlers, which Cluster 251 *did*
session-mount — that is precisely why *their* `ensure_acting_member` is load-bearing). The
only callers are bearers, which are **act-as-any by design** (the 202/203 orchestrator model
we deliberately kept). So there is no session-exploitable gap.

I kept the three `ensure_acting_member` guards as **defensive consistency** (a strict no-op
for every current caller; they pin a session to self *if* these are ever `/ui/api`-mounted
like 251) and reframed the comments truthfully. The regression test documents the real truth
(`legacy_inbox_and_mentions_are_bearer_only_not_session_reachable`: a session cookie → 401).

**Lesson:** a research agent flagged a missing guard by comparing to a sibling *without
checking the auth layer that makes the sibling reachable*. Verifying reachability — not just
"is the guard present?" — is the difference between a real vuln and a no-op. This is exactly
why the plan was "verify before folding."

## Deferred from 315 (with reason)

- **Outbox `list_pending` `FOR UPDATE SKIP LOCKED`** (K8) → its own cluster. The two-replica
  double-publish is real but bounded (idempotent by `log_id`), and **a naive `FOR UPDATE` on
  the pooled SELECT is a no-op false fix** — the lock releases when the statement's implicit
  tx ends, and the relay publishes+marks outside any tx. A correct fix needs a lease column
  (migration) or a held transaction across the batch publish + a multi-replica test. Not a
  hasty 315 line.
- **K3/K4** (resume-secret getter panic → invariant; `AUTH_DISABLED` test-secret ack) —
  behaviour-changing, low urgency; deferred.

## Test evidence

`ui_channels_e2e` (incl. the new reachability test) + `inbox_e2e` + `notifications_inbox_e2e`
+ `event_emission_e2e` green; strict clippy (`-D unwrap_used -D expect_used`) + `--all-targets`
+ `--no-default-features` bootstrap-strip clean; mdbook linkcheck green.

## Forward look

315 opens the sweep program. Next: **316** honesty scrub + no-clone image, **317** Bet 2
snippet pack, **318** token-pack evidence, then the **fidelity + context flagship arc**. The
public launch stays gated on the maintainer's go.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Plan folded into [[Open Work]] from
the 2026-08-28 research sweep (`docs/Undeniable Final.md` is the strategy "why").
