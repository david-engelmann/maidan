# Cluster 138.0 retro — Global audit + reindex controls in the operator console

> Tag **`v138.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Completes the operator-console arc (137–138).

## What shipped

- **Global audit section** (`panel-operator`): a limit + "Load global audit"
  that GETs the top-level `/operator/audit` with a bearer (cap
  `audit:read-global`) and renders `time · action · actor · target` rows.
  Bearer-only by design; with no token the UI explains the requirement.
- **Reindex section**: "Reindex this workspace" (`POST {workspace_id}`,
  `workspace:write` — works on a login), "Reindex system-wide" (`POST {}`,
  `token:admin` bearer), and "Check job" (poll by id → status / processed /
  failed / error).
- **Routes** (write router): `POST /ui/api/operator/reindex-embeddings` +
  `GET /ui/api/operator/reindex-embeddings/:job_id`, reusing the tested
  `reindex_ops::{start_reindex_embeddings,get_reindex_embeddings_job}`.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Live reindex-status stream | Poll-by-id matches the rest of the console. |
| n/a | An audit drill-down view | The row summary suffices; per-workspace audit drill-down already lives in Admin. |
| n/a | `/ui/api/operator/audit` proxy route | The token path hits the top-level route directly; a session can't carry the cap, so a proxy would only ever 403. |

## Surprises

- **The reindex status GET had to go on the *write* router.** A
  workspace-scoped job's read requires `workspace:write` (not
  `workspace:read`), so a plain read-session 403s — the only `/ui/api` GET
  so far that needs the write middleware. Captured in a why-comment in
  `app.rs`.
- **No proxy route for the global audit at all.** Since a session can never
  hold `audit:read-global`, the cleanest path is to call the top-level
  `/operator/audit` directly when a bearer is present and skip a proxy that
  would only return 403.

## Decisions

- **Gate each control by the cap it truly needs, and label it.** Rather than
  hide the elevated controls, surface them with a clear note (bearer /
  token:admin) and degrade honestly when the token is absent.
- **Reuse handlers under `/ui/api`** (reindex) / **call top-level directly**
  (audit) — no new backend logic.

## Capability table extension

| Capability | Where |
|------------|-------|
| Load cross-workspace global audit in `/ui` (bearer) | `static/index.html`, top-level `/operator/audit` |
| Trigger + poll embedding reindex in `/ui` (workspace = session; global = token:admin) | `static/index.html`, `/ui/api/operator/reindex-embeddings[/:job_id]` |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; the
  `ui_js_contract` guard covers references, the reindex/audit e2e cover the API.

## Forward look

The operator-console arc (137 deliveries/DLQ + 138 audit/reindex) is
complete. Further UI work is open-ended; reassess against [[Open Work]] /
[[Remaining Work]] before opening 139.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
