# Cluster 366 retro — Wave 1 #14: four independent tracks

Wave 1 #14 is not one feature — the backlog flags it as "four bullets, not one
cluster": **T6 legal-hold**, **H15 OTel feature-gate**, **N1 web-push**, and
**SCIM-as-OIDC-P3**. They share nothing but a priority rank. So this cluster ran
them as **four independent PRs to `main`** (no stacking, no cascade), pipelined so
each PR's CI ran while the next was built — the fastest structure given how long CI
takes.

## What shipped

- **366.1 (#717) — T6 legal hold.** `maidan_legal_holds` (pg 0070 / sqlite 0069):
  a hold on a workspace preserves its data against every deletion path. The
  Cluster-186 retention `prune_events` now excludes held workspaces' events (an
  in-SQL `NOT IN` subquery — zero sweeper/trait churn), audit pruning freezes while
  any hold is active, and purge/erase are refused with `409`. REST place/lift/get
  (`token:admin` — a higher bar than the `workspace:write` that purges) +
  `/operator/legal-holds`.
- **366.2 (#718) — H15 OTel feature-gate.** OpenTelemetry (OTLP trace + metrics)
  is now a default-on cargo feature `otel` on `maidan-observability`, forwarded by
  `maidan-server` with `default-features = false`. `--no-default-features` compiles
  the OTLP/tonic stack out entirely (plain tracing + Prometheus scrape stay) — and
  the existing `bootstrap compile-time strip` job exercises that path for free.
- **366.3 (#719) — N1 web-push.** A member who closed their tab (no live WS) now
  gets a Web Push notification. `maidan_push_subscriptions` (pg 0071 / sqlite 0070)
  + hand-rolled VAPID (ES256, RFC 8292) and aes128gcm payload encryption (RFC 8291
  over 8188) with RustCrypto — no openssl, only `aes-gcm` was a new dep. The router
  delivers iff the member has no live WS (last-seen gate), pruning a `410 Gone`
  subscription. REST register/list/delete.
- **366.4 (#720) — SCIM-as-OIDC-P3.** A minimal RFC 7643/7644 SCIM 2.0 endpoint at
  `/scim/v2/` so an IdP can provision/deprovision members: `ServiceProviderConfig`
  + `Users` create/read/list-with-`userName eq`-filter/replace/patch/delete.
  `maidan_scim_users` (pg 0072 / sqlite 0071) tracks `externalId` + `active`;
  deactivation/delete revoke the member's API tokens. Gated `token:admin`, outside
  the OpenAPI doc + capability-map (the `/mcp` precedent).

## Decisions

- **Four independent PRs, not a stack.** Because the tracks touch disjoint files,
  each targeted `main` directly. That meant three PRs' CI runs overlapped, and each
  merged as it greened — no cascade, no rebase gymnastics. It's the right shape when
  the work is genuinely independent.
- **Legal-hold protects the event log precisely, audit coarsely.** The event log
  (the "immutable log" the backlog names) is workspace-tagged, so held workspaces'
  events are exempted per-workspace. `maidan_audit` has no `workspace_id`, so audit
  pruning freezes globally while any hold exists — coarse but safe (never deletes
  evidence). Both via in-SQL subqueries: no sweeper, trait, or signature change.
- **OTel as a *cargo* feature, wired through the server's defaults.** "Feature-gate
  (crates already pinned)" reads as making the pinned OTel crates compile-optional.
  Forwarding the feature through `maidan-server`'s defaults means the required
  bootstrap-strip job already covers the no-otel build — no new CI job for real
  coverage.
- **Web-push crypto hand-rolled on RustCrypto.** `p256`/`hkdf`/`sha2`/`base64` were
  already in the tree transitively, so only `aes-gcm` was new — avoiding openssl and
  any cargo-deny churn. The round-trip test decrypts through an *independent*
  UA-side implementation, so it validates the RFC 8291 key-derivation ordering, not
  just self-consistency.
- **SCIM stays out of OpenAPI + the capability-map.** Like `/mcp`, SCIM has its own
  schema and error envelope; each handler enforces `token:admin` inline. This kept
  the openapi↔map bijection + capability-matrix contracts untouched.

## Surprises

- **The missing `pub mod web_push;` (a real CI catch).** I staged `web_push.rs` but
  forgot `lib.rs` in the 366.3 commit — it built locally (the working tree had the
  mod decl uncommitted) but CI failed everywhere with `could not find web_push in
  the crate root`. A one-line amend + rebase onto main fixed it. Lesson: a new
  module's `mod` declaration is a separate file from the module itself — stage both.
- **SCIM's `userName eq` filter is form-urlencoded.** `urlencoding::decode` doesn't
  turn `+` into a space, so a filter with `+`-encoded spaces silently failed to
  parse; normalize the literal `+` before decoding (unit-tested).
- **The `token:admin` FK trap, again.** Legal-hold place (`placed_by`) and the SCIM
  provisioning flow both persist a real member — the auth-enabled `AppState::new`
  path with a minted token, not the nil-member `for_tests` bypass.

## Test evidence

- Store (both backends): `legal_holds` (+ retention exemption), `push_subscriptions`,
  `scim_users`.
- Crypto: `web_push` VAPID-JWT-verifies + encrypt-round-trips; the observability
  otel-on/off builds + tests; the SCIM filter parser.
- Server e2e: `legal_hold_e2e` (purge 409 while held), `web_push_e2e` (offline→sent /
  present→skip / Gone→prune), `scim_e2e` (full lifecycle + deactivation token
  revocation + `token:admin` gate). Bijection + capability-matrix + backend-parity
  green; the bootstrap-strip `--no-default-features` build (no otel) green.

## Forward look

**Wave 1 #14 is complete.** Deferred (logged): reactions/votes in the workspace
export; legal-hold audit-selectivity (needs `workspace_id` on `maidan_audit`); a
`/ui` web-push subscribe button + a durable push-retry queue; SCIM Groups +
userName/displayName rename (members are immutable — needs an `update_member`) +
complex filters. **Next: Wave 2** — H1 (AG-UI) is #17.

## Acknowledgements

Four PRs (#717, #718, #719, #720) plus this retro, built on the Cluster-186
retention, Cluster-247 mail-transport-attach, and `/mcp`-style off-contract-router
patterns.
