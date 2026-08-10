# Cluster 182.0 — security: audit-log coverage for credential + membership mutations

**Theme:** Arc A (security & correctness), part 4 — extend the audit trail to the
security-sensitive state changes that weren't being recorded.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v182.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `crate::audit::record` — best-effort audit helper (loud log on failure) | `maidan-server/src/audit.rs` (new) |
| `token.mint` / `token.revoke` audited | `routes/token.rs`, `session/handlers.rs` (OIDC first-admin) |
| `app_token.mint` / `app_installation.revoke` audited | `apps.rs` |
| `channel_member.add` / `channel_member.remove` audited | `routes/channel.rs` |
| `message.purge` audited | `routes/message.rs` |

## Why

The audit trail (`maidan_audit`, surfaced by `GET /workspaces/:id/audit` +
`GET /operator/audit`) only recorded operator replay/reindex actions and
workspace lifecycle. The security-critical mutations — **who minted or revoked a
bearer token, who granted or revoked private-channel membership, who hard-purged
a message** — left no trace. A research sweep flagged this as the audit gap.

## The fix

- A small `audit::record(state, NewAuditEvent)` helper writes best-effort: on a
  store error it emits a loud `tracing::error!(target: "audit", … "audit.write_failed")`
  and lets the operation succeed. An audit write must never break the operation
  it records — a failed insert must not, e.g., cost a caller the token secret
  that only exists in the response body. (Transactional audit is the dual-write
  concern for Cluster 184.)
- Each credential/membership/purge handler records a typed action
  (`token.mint`, `token.revoke`, `app_token.mint`, `app_installation.revoke`,
  `channel_member.add`, `channel_member.remove`, `message.purge`) with the actor,
  a `target_kind`/`target_id`, and metadata (workspace, subject member,
  capabilities, etc.).

## Deliberately out of scope: 401/403 denial auditing

The sweep also listed "audit 403 denials". **Declined for the audit table.** A
rejected request stream is attacker-controlled, so table-level denial auditing is
an unbounded write amplifier (spray invalid requests → fill `maidan_audit`, a
DoS + noise vector). Denials are already observable through structured request
logs + metrics; that's the right home for high-cardinality, adversary-driven
signal. Logged in [[Open Work]].

## Exit criteria

- Token mint/revoke, app-token mint/installation-revoke, channel membership
  grant/revoke, and message purge each write an audit row visible in the
  workspace + operator audit lists — **met**.
- `v182.0.0` tagged.

## Verification & limits

- `audit_coverage_e2e::token_mint_revoke_and_membership_grants_are_audited`
  (auth enabled): mint → revoke → grant over HTTP, then assert `token.mint`,
  `token.revoke`, `channel_member.add` appear in `GET /workspaces/:id/audit` with
  the right `target_kind` + subject. Existing `workspace_audit` / `operator_audit`
  / `workspace_purge` suites stay green.
- Limit: best-effort write means a persistent audit-store outage drops rows
  (logged, not failed) — acceptable until 184's dual-write work; denial auditing
  intentionally excluded (see above).

## References

- [[Retros/Cluster 182.0]]; `maidan-server/src/audit.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc A).
