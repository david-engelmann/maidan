# Cluster 138.0 — Global audit + reindex controls in the operator console

**Theme:** Complete the "Operator" tab (started in 137) with the two
elevated controls — the cross-workspace global audit and reindex-embeddings
— each gated by the capability it actually needs.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v138.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Routes (app.rs)** | `POST /ui/api/operator/reindex-embeddings` (write) + `GET /ui/api/operator/reindex-embeddings/:job_id` (write router) → `reindex_ops::*`. Global audit uses no proxy — the UI hits the top-level `/operator/audit` with a bearer. |
| **UI (index.html)** | Two `<details>` in `panel-operator`: **Global audit** (limit + load, bearer-only) and **Reindex embeddings** (reindex this workspace / system-wide + poll-by-job-id). |

## Capability model (why each control is gated as it is)

| Control | Endpoint | Cap | Works on a session? |
|---------|----------|-----|---------------------|
| Global audit | `GET /operator/audit` | `audit:read-global` | **No** — bearer with the cap only. |
| Reindex this workspace | `POST /operator/reindex-embeddings {workspace_id}` | `workspace:write` | **Yes** — write session grants it. |
| Reindex system-wide | `POST /operator/reindex-embeddings {}` | `token:admin` | **No** — bearer only. |
| Reindex status | `GET /operator/reindex-embeddings/:job_id` | `workspace:write` (scoped) / `token:admin` (global) | **Yes** for a scoped job. |

The UI degrades honestly: with no token, "Load global audit" explains the
requirement instead of firing a doomed request; "Reindex system-wide" warns
before posting.

## Non-goals

- A live job-status stream — poll-by-id matches the rest of the console.
- An audit detail/drill-down view — the row (time · action · actor · target)
  is the operator summary; the per-workspace audit drill-down lives in Admin.

## PR ladder (actual)

| # | Title |
|---|--------|
| 138.0.1 | `feat(ui): global audit + reindex controls in the Operator tab` (#366) |
| 138.0.retro | `docs(retro): Cluster 138.0 + v138.0.0 tag prep` |

## Exit criteria

- Global audit loads (bearer) and reindex starts/polls in the UI; routes
  wired under `/ui/api`; guard green — **met**.
- `v138.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; `fmt`/`clippy` clean. Per the
  UI track's standing limit, JS *behavior* is inspection-verified (no browser).

## References

- [[Retros/Cluster 138.0]]; [[Clusters/Cluster 137.0]]; `static/index.html`
  (`loadGlobalAudit`/`startReindex`/`pollReindex`), `app.rs`,
  `reindex_ops.rs`, `routes/workspace.rs` (`list_global_audit`).
