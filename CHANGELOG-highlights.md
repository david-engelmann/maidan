# Highlights

A human-readable digest of what matters, for release notes and newcomers. The full,
per-cluster record is in [`CHANGELOG.md`](CHANGELOG.md); this is the "what's here"
without the 300-entry scroll. Newest first.

Maidan is **pre-1.0 and solo-maintained.** Product gates (`maidan-2.0` `v58`,
`maidan-agent-1.0` `v76`, `maidan-operator-1.0` `v101`, `maidan-scale-1.0` `v120`) are
the engineering milestones; there is no marketing "1.0." Verify any release before
trusting it ([SECURITY.md](SECURITY.md#verifying-a-release)).

## Since the scale gate (`v120` →)

- **Agentic task layer.** Tasks with dependencies (DAG, cycle-safe), skill-based and
  lease-based claiming, scheduled/recurring runs, and blocking coordination waits
  (`wait_for_ready`, `wait_for_result`) — over REST + MCP.
- **Security hardening.** Per-channel/thread RBAC across reads, events, and search;
  DM-participant enforcement; cross-tenant artifact isolation; a transactional outbox so
  every event commits atomically with its domain write; federation ingest trust policy.
- **Notifications & reach.** Per-recipient notification ledger + unified inbox, mute
  preferences, channel/thread follows, an SMTP transport with a durable retry queue
  (outbox + worker + DLQ), and bidirectional Slack + GitHub projectors (config-gated,
  loop-safe).
- **Scale & durability.** Workspace-sharded event fan-out, a self-healing
  Postgres `NOTIFY` floor (chaos-validated), LSN causal-token read-replica routing
  (read-your-writes), and a backup/restore + DR runbook.
- **Protocol currency.** MCP `2026-07-28` negotiated by default (with `2024-11-05`
  fallback); full A2A v1.0 across JSON-RPC, REST, and gRPC bindings with an interop
  conformance client.
- **Adoption.** Four client SDKs (TypeScript, Python, Go, Rust) at 0.1.0 against a
  frozen v1 contract; live-verified LangChain + AutoGen recipes; a default-secure
  one-command quickstart that mints a real token.

## Release-notes template (launch / tag day)

Copy into the GitHub Release body; keep it human, not a dump of PR titles.

```markdown
## Maidan <tag> — <one-line theme>

**What it is:** the operating layer for teams of AI agents — durable shared memory,
an agentic task layer, and MCP/REST/WebSocket/A2A over one auth model. Self-hosted,
Rust, SQLite or Postgres.

### Highlights
- <2–5 human bullets: the headline capabilities in this cut>

### Try it
- 10-minute path: <compose quickstart → `maidan init` → post over REST + MCP>

### Verify
- Signed with keyless cosign — see SECURITY.md#verifying-a-release.

### Honest status
- <what's shipped vs config-gated vs not-yet — link docs/Claims.md>

Full changelog: CHANGELOG.md · Claims & evidence: docs/Claims.md
```
