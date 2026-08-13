# Cluster 204.0 retro — a SHA is no longer a cross-tenant key

> Tag **`v204.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 3.

## What shipped

- A `maidan_artifact_refs` per-workspace access link over the deduped,
  content-addressed blob store: a ref is written on upload and required on fetch,
  so a caller can only download artifacts its workspace uploaded (or re-uploaded).
  Cross-tenant fetch → 404. Dedup preserved.

## Surprises / decisions

- **Dedup is exactly what made it a vuln, and exactly what the fix must preserve.**
  Content-addressing stores each blob once across all tenants — great for storage,
  but it means the SHA *is* the only key, and the key was global. The fix can't
  add `workspace_id` to the blob (that would un-dedup it); it has to be a *side*
  table of (workspace, sha) grants layered over the shared blob. Two workspaces
  uploading the same file each get a grant; the blob stays single. Getting that
  framing right was the whole design.
- **404, not 403.** Returning 403 ("you can't access this") confirms the SHA
  exists somewhere — the dedup oracle in a different disguise. 404 makes "no grant"
  and "no such artifact" indistinguishable, so a cross-tenant SHA reveals nothing.
- **The migration silently didn't run — because migrations aren't auto-discovered.**
  The `.sql` files were correct, the code compiled, tests started — and the upload
  500'd with "database error" because `maidan_artifact_refs` didn't exist. The
  runner (`migrate.rs`) is a hardcoded `include_str!` + `apply_*` list; a new file
  must be registered in both places, both backends. Cost one debug round; now in
  memory `maidan-migration-register` so it doesn't recur.
- **Backfill from the uploader's workspace.** Without it, turning on the ref check
  would lock every existing workspace out of its own artifacts. A one-line SQL
  backfill (`JOIN maidan_members ON id = uploaded_by`) restores access for the
  common case; artifacts with no uploader, or referenced-but-not-uploaded, are the
  documented edge.

## Decisions

- **Ref-counted blob GC deferred, on purpose.** The security fix is the access
  gate; deleting the shared blob only when its last workspace-ref goes is a
  storage-GC concern (and the dedup+purge interaction pre-existed this cluster).
  The ref FK's `ON DELETE CASCADE` cleans up a purged workspace's refs; blob GC is
  logged in Open Work.

## Capability table extension

| Change | Where |
|--------|-------|
| Per-workspace artifact access links (`maidan_artifact_refs`) | `store/*/artifacts.rs` + `routes/artifact.rs` + migrations |

## Risks identified + still open

- **Net additive** — dedup unchanged, existing access preserved by backfill, bypass
  unaffected. Open: ref-counted blob GC (a shared blob can outlive all its
  workspace refs); a message-metadata backfill for artifacts referenced but not
  uploaded by a workspace.

## Forward look

Program A's last two clusters are the heaviest: **205** transactional outbox
(atomic domain-write + event-append — the twice-deferred multi-cluster refactor;
will be scoped incrementally, `*_with_event` methods on the simple mutations
first), and **206** federation ingest trust policy + an optional RLS spike. Then
Programs B (agentic orchestration), C (notifications & reach), D (scale &
durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Closes Threat-Model T5.
