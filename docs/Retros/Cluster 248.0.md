# Cluster 248.0 retro — where the email goes

> Tag **`v248.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 12** — Arc I.

## What shipped

- `maidan_member_emails` (pg 0046 / sqlite 0045; `member_id` PK, `email`) +
  `MemberEmail` model + store `set` / `get` / `delete`, both backends. The
  recipient-address prerequisite for email delivery — storage only.

## Surprises / decisions

- **A table, not a member column — to dodge the row ripple.** A member's email is
  conceptually member metadata, and the tidy instinct is a column on
  `maidan_members`. But that column would have to be threaded through every
  `row_to_member` and every member `SELECT`/`RETURNING` across both backends (the
  `maidan-schema-column-ripple` trap that has cost CI round-trips before). A separate
  one-row-per-member table is zero-ripple, and it generalizes (a `phone` for SMS
  slots into the same shape) — so the "awkward" separate table is the right call.
- **Dumb store, validate at the edge.** The store persists whatever string it's
  handed; the SMTP transport already parses the `to` mailbox on send, and the REST
  layer (249) can pre-validate. Duplicating email-format rules in the store would just
  be a second place to get RFC 5322 subtly wrong.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_member_emails` + `MemberEmail` + set/get/delete store | `migrations/*`, `models.rs`, `store/*/member_emails.rs` |

## Risks identified + still open

- None — a new table off every existing path.

## Forward look

**249** wires it: an email-delivery preference + the notification router (or a
delivery step) looking up a member's address and calling `MailTransport::send` when
they've opted in, plus the REST/MCP to set/clear the address + preference. Then
scheduled digests, presence-aware routing (needs durable `last_seen`), and the `/ui`
notification center. Then Program D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 247.0]].
