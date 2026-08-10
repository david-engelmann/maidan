# Cluster 182.0 retro — the audit trail now records credential + membership changes

> Tag **`v182.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc A (security & correctness), part 4.

## What shipped

- A `crate::audit::record(state, NewAuditEvent)` best-effort helper.
- Audit writes on every credential/membership/purge mutation: `token.mint`,
  `token.revoke` (incl. the OIDC first-admin session mint), `app_token.mint`,
  `app_installation.revoke`, `channel_member.add`, `channel_member.remove`,
  `message.purge` — each with actor, `target_kind`/`target_id`, and metadata.
- `audit_coverage_e2e` proving the mint→revoke→grant path lands in
  `GET /workspaces/:id/audit`.

## Surprises

- **Auditing after a mint can't be allowed to fail the mint.** The natural
  pattern (`.append_audit(…).await?`) that the pre-existing operator-replay audit
  sites use would, on an audit-store hiccup, return 500 *after* the token row was
  already created — and the secret only exists in the response body, so the
  caller loses it forever. That pushed the design to a best-effort helper with a
  loud `audit.write_failed` log. The audit trail is a security *record*, not a
  transactional participant.
- **A double-move hid in the obvious code.** `capabilities` feeds both the audit
  metadata and the response body; `serde_json::json!` moves its value, so the
  metadata arm needed `.clone()` — the compiler caught it, but it's a reminder
  that `json!` takes ownership.

## Decisions

- **Declined table-level 401/403 denial auditing** (the sweep asked for it). A
  rejected request stream is attacker-controlled, so writing an audit row per
  denial is an unbounded write amplifier — a DoS + noise vector against the audit
  table itself. Denials belong in structured logs + metrics (already emitted).
  Logged in Open Work.
- **Best-effort, not transactional.** A persistent audit outage drops rows
  (logged, not failed). Making the audit write atomic with its mutation is the
  dual-write concern for Cluster 184; conflating them here would have been scope
  creep on the wrong layer.

## Capability table extension

| Change | Where |
|--------|-------|
| Audit trail covers token mint/revoke, app-token mint / installation revoke, channel membership grant/revoke, message purge | `maidan-server/src/audit.rs` + handlers |

## Risks identified + still open

- **Net risk-reducing** — additive audit writes, no behaviour change to the
  mutations themselves. Open: best-effort drop on audit-store outage (→ 184);
  denial auditing intentionally excluded (→ logs/metrics).

## Forward look

Arc A closes with: default-on rate limits + request body-size cap (183), then
dual-write atomicity (184) — which also upgrades this cluster's best-effort audit
into a transactional one.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
