# Cluster 213.0 — transactional outbox: A2A ingest, member + workspace creation

**Theme:** Program A (security & correctness round 2), part 12 — migrate the A2A
ingest post and the member / workspace creation events to the transactional-outbox
pattern.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v213.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| A2A ingest post reuses `post_message_with_event(new, None)` | `a2a_agent.rs` |
| `create_member_with_event` (`MemberJoined`) | `store/{postgres,sqlite}/members.rs`, `store.rs`, `*/mod.rs`, `routes/member.rs` |
| `create_workspace_with_event` (`WorkspaceCreated`) | `store/{postgres,sqlite}/workspaces.rs`, `store.rs`, `*/mod.rs`, `routes/workspace.rs` |

## Why

After the message paths (210–212), the remaining `publish()` callers are the A2A
ingest post and the member / workspace / reference / artifact creation events. This
cluster takes the first three: A2A ingest (structurally a DM post — a plain insert
+ `MessagePosted`, so the Cluster 210 method fits directly) and the two
identity/creation events.

## The change

- **A2A ingest** swaps `post_message` + `publish(MessagePosted{…})` for
  `post_message_with_event(new, None)` + `publish_stored` — no new store method
  (it's the DM-post shape). The route drops its hand-built event and its
  `resolve_thread_context`-derived `channel_id`.
- **member / workspace** gain `create_member_with_event` / `create_workspace_with_event`
  (both backends) — insert the row and append `MemberJoined` / `WorkspaceCreated`
  in one tx. No scope resolution needed (the created entity *is* the event's
  subject). The routes (both `#[cfg(feature = "bootstrap")]`) call them +
  `publish_stored`; their bootstrap-gated `publish` / `Utc` imports go with the
  change.

## Exit criteria

- An A2A post / member create / workspace create and its event commit atomically —
  **met**.
- `v213.0.0` tagged.

## Verification & limits

- `event_log::create_workspace_and_member_with_event_append_atomically` (store):
  create-workspace and create-member each commit the row + a durable
  `WorkspaceCreated` / `MemberJoined`.
- Behaviour-preserving: `a2a_protocol_e2e`, `bootstrap_e2e`, `http_crud_e2e`,
  `event_emission_e2e`, `federation_e2e` + the store suite (both backends) green.
  The `bootstrap-strip` (`--no-default-features`) build is checked locally (the
  cfg-gated `publish_stored` import).
- **Limit (tracked):** `publish()` now serves only the **reference** and **artifact**
  events (+ the federation relay). Those are the last domain-mutation targets.

## References

- [[Retros/Cluster 213.0]]; `store/*/{members,workspaces}.rs`, `a2a_agent.rs`.
  Program: [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Continues
  [[Retros/Cluster 212.0]].
