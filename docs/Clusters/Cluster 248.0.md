# Cluster 248.0 — member delivery emails (store foundation)

> **Program C (notifications & reach), part 12** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v248.0.0`**. No new gate tag.

## Goal

The recipient-address prerequisite for email delivery: a store for a member's
delivery email, so Cluster 249's wiring has somewhere to look up where to send.

## Scope

| Change | Where |
|--------|-------|
| `maidan_member_emails` table (pg 0046 / sqlite 0045; `member_id` PK, `email`) | `migrations/*`, `migrate.rs` |
| `MemberEmail` model | `maidan-types/src/models.rs` |
| Store `set_member_email` (upsert) / `get_member_email` / `delete_member_email`, both backends | `store.rs`, `store/{sqlite,postgres}/member_emails.rs`, `store/*/mod.rs` |

## Design decisions

- **A separate table, not a column on `maidan_members`.** Adding `email` to the
  members table would ripple through every `row_to_member` / member `SELECT`
  crate-wide (the `maidan-schema-column-ripple` lesson). A one-row-per-member
  `maidan_member_emails` table is zero-ripple and leaves room for other contact
  channels (a phone column for SMS later) without touching the member row.
- **Store the address as-is; validate at send.** The store keeps whatever string it's
  given; the SMTP transport (Cluster 247) validates the `to` mailbox on send, and the
  249 REST layer can pre-validate. Keeping the store dumb avoids duplicating
  address-parsing rules.
- **set / get / delete.** Upsert to set (one address per member), `get` → `Option`,
  `delete` to remove — the trio a "manage my delivery address" surface (249) needs.
- **Foundation only.** No delivery wiring; the transport still sends nothing until 249.

## Non-goals / deferred

- **Wiring** (Cluster 249) — an email-delivery preference + the router/worker looking
  up the address and calling `MailTransport::send`, plus REST/MCP to set/clear it.
- Address verification (a confirm-your-email flow) — a later refinement.

## Risks

- Migration registration — covered by the both-backend store test + `dialect_parity` /
  `concurrent_migrations`.
