# Cluster 159.0 — channel membership model (RBAC part A)

**Theme:** First cluster of the flagship **channel/thread RBAC** arc. Land the
membership substrate only — additive, no enforcement — so the risky enforcement
flip (Cluster 160) sits behind an already-present, already-tested table.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v159.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `maidan_channel_members` table | `migrations/postgres/0032`, `migrations/sqlite/0031` |
| `ChannelMember` + `ChannelMemberRole {Member, Admin}` | `crates/maidan-types/src/models.rs` |
| Store methods: add (idempotent upsert) / remove / list / `channel_is_member` | `crates/maidan-store/src/store.rs` + `{postgres,sqlite}/channel_members.rs` |
| Both-backend round-trip test | `crates/maidan-store/tests/common/mod.rs` (+ roundtrip callers) |

## Why

Authorization is workspace-flat (the #1 production-readiness finding): any
`message:post` token can read/write every channel and thread, including private
ones (private enforcement lives only in the subscribe fan-out and is
self-asserted). RBAC needs a place to record who belongs to a channel. This
cluster adds exactly that and nothing else, so it ships with zero behavior
change and zero existing-test churn.

## Non-goals

- **Enforcement** — Cluster 160 (`ensure_channel_access`).
- **Membership management API** — Cluster 161 (`channel:admin`).
- **Auto-adding the channel creator** — happens at private-channel-create time in
  Cluster 160 (where creation becomes access-relevant).

## PR ladder (actual)

| # | Title |
|---|--------|
| 159.0.1 | `feat(store): channel_members model + store + migration` (#408) |
| 159.0.retro | `docs(retro): Cluster 159.0 + v159.0.0 tag prep` |

## Exit criteria

- Table + types + store methods on both backends; round-trip test green on
  sqlite and (CI) real Postgres — **met**.
- `v159.0.0` tagged after retro.

## Verification & limits

- `run_channel_members_scenario` (shared harness) runs on both backends
  (`sqlite_roundtrip`, `postgres_roundtrip`): add → is_member → list →
  idempotent-upsert-preserves-`created_at` → remove.
- CI note: the first `#408` run hit a **GitHub Actions service outage** ("failed
  to resolve action download info") on `bootstrap-strip` + `otlp smoke` during
  job setup — pure infra, cleared on rerun; the substantive `integration
  (testcontainers)` job passed first time.

## References

- [[Retros/Cluster 159.0]]; scratchpad `rbac-plan.md` (full 159–161 design);
  `group_dm` join-table pattern. Program: [[Roadmap]] + memory `maidan-next-arc-program`.
