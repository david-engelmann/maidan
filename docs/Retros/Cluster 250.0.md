# Cluster 250.0 retro — members register their email

> Tag **`v250.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 14** — Arc I.

## What shipped

- `PUT`/`GET`/`DELETE /members/:id/email` — set / read / clear a member's delivery
  address, `workspace:read` + self-only. With 249's router wiring, the email feature
  now works end-to-end over REST: a member registers an address → their notifications
  arrive by email (when SMTP is configured).

## Surprises / decisions

- **The opt-in *is* the address CRUD.** Since 249 made "has an address" the opt-in,
  no separate enable/disable endpoint is needed — `PUT` opts in, `DELETE` opts out.
  The controls fall straight out of the model.
- **A `@` check, not an RFC parser, at the edge.** 248 chose "store dumb, validate at
  send." But a member typing a bad address and silently never getting email is poor
  UX, so `PUT` does a minimal `contains('@')` + length check → `400`. Full validation
  still lives in the transport (lettre's mailbox parse on send); the edge check just
  catches obvious typos immediately without coupling the route to lettre.
- **Self-only, `workspace:read` — the inbox/prefs cap model.** Managing your own
  delivery address is self-config, same as notification prefs and follows; a bearer is
  the act-as-any orchestrator.

## Capability table extension

| Change | Where |
|--------|-------|
| `PUT`/`GET`/`DELETE /members/:id/email` | `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## Risks identified + still open

- None new. Email is usable end-to-end over REST; MCP email tools are an optional
  low-value follow-up.

## Forward look

The email-transport thread of Arc I is substantially done (247 transport → 248
address store → 249 router wiring → 250 REST). Remaining Arc I: scheduled **digests**
+ unread rollups, **presence-aware routing** (needs durable `last_seen` — presence is
in-memory only today), and the **`/ui` notification center**; optionally MCP email
tools for parity. Then **Program D (scale & durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 249.0]].
