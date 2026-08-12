# Cluster 202.0 — security: session-bound acting identity (anti-spoofing)

**Theme:** Program A (security & correctness round 2), part 1 — close a
session-impersonation vulnerability. Opens the new four-program arc from the
2026-08-12 research sweep.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v202.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `ensure_acting_member(auth, claimed)` — a session caller may act only as its own member | `routes/mod.rs` |
| Applied to every member-attributed write surface | `message.rs`, `dm.rs`, `group_dm.rs`, `social.rs`, `thread.rs` |
| Unit test (all branches) + adversarial e2e on a newly-guarded surface | `routes/mod.rs`, `tests/ui_channels_e2e.rs` |

## Why

Maidan's write handlers take the acting member as a **caller-supplied** field
(`author_id`/`actor_id`/`editor_id`/voter `member_id`). For a **bearer token**
that is intentional — the orchestrator model lets a service token act on behalf
of any member in its workspace. But a **session** caller (the `/ui`, a browser
OIDC login with no API token) has a fixed identity and must not be able to pose
as someone else. Only `post_message` enforced that (`message.rs:34`); every other
member-attributed write trusted the field. So a signed-in `/ui` user could post
DMs and group-DM messages, edit messages, cast votes, add/remove reactions,
pin/unpin, and transition/assign/claim/renew threads **as any other member** —
a straightforward impersonation vulnerability across the whole write surface.

## The fix

Extract the guard `message.rs:34` had into a shared helper:

```rust
pub(crate) fn ensure_acting_member(auth: &AuthContext, claimed: MemberId) -> ApiResult<()> {
    if !auth.bypass && auth.token_id.is_none() && claimed != auth.member_id {
        return Err(ApiError::Forbidden("a session caller may only act as its own member".into()));
    }
    Ok(())
}
```

and apply it on every member-attributed write. The precise part is guarding the
**actor**, not a target: a *mention*'s `member_id` is the *mentioned* party (you
may mention anyone) and *assign*'s `assignee_id` is the assignment target (you may
assign to anyone) — only `assign`'s `actor_id` is the actor. Those targets are
left unguarded; the actor fields are guarded.

## Exit criteria

- A session caller may act only as its own member on every write surface; bearer
  and bypass are unchanged — **met**.
- `v202.0.0` tagged.

## Verification & limits

- Unit test `ensure_acting_member_blocks_session_spoof_only`: session-mismatch →
  403, session-self → ok, bearer → ok (act-as-any), bypass → ok.
- `ui_channels_e2e::session_cannot_react_as_another_member`: a real OIDC session
  reacting as another member is `403`, as itself is allowed — proving the guard
  is wired on a *newly-guarded* surface, not only `post_message` (whose existing
  spoof test still passes).
- All affected-surface suites (thread assignment, reactions/pins, DM, group DM,
  edit history, ui collab) stay green — bearer/bypass paths are unchanged.
- Limit: the guard covers the HTTP write handlers; it does not change the bearer
  orchestrator model (act-as-any is intentional). A follow-up could decide+document
  whether a workspace bearer should be allowed to post into arbitrary private DMs
  as any participant.

## References

- [[Retros/Cluster 202.0]]; `routes/mod.rs`. Program: [[Roadmap]] +
  [[Open Work]] + memory `maidan-next-arc-program` (Program A, from sweep
  `wf_b8cdaaa2-be4`). First cluster of the new four-program arc.
