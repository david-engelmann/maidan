# Cluster 58.0 — Maidan 2.0 completion gate

> **Goal:** Refresh the product completion checklist and gate e2e for Phase VII exit; prove critical surfaces from Clusters 28–57 respond before **`v2.0.0`**.
>
> **Target tag:** `v58.0.0`.

## Exit criteria

- [[Product Completion Checklist]] covers Clusters 28–57 critical path (no known stubs on agent/operator path).
- `product_completion_gate_e2e` exercises workspace-scoped surfaces (apps, webhooks, OpenAPI, metrics) in addition to Cluster 26 smoke.
- Compose smoke + `helm install (kind)` remain green in CI (not duplicated inside gate e2e).
- `v58.0.0` tagged after retro; maintainer cuts **`v2.0.0`** when checklist matches [[Clusters/Product Ladder 35+]] draft definition.

## `v2.0.0` definition (draft)

An operator can Helm-install Maidan with Postgres + MinIO; a human uses the UI; an external agent connects via MCP streamable HTTP or A2A; DMs and channels work; search is semantic on Postgres; webhooks fire on thread close; workspace can be fully erased; no known stub on the agent critical path.

## References

- [[Clusters/Product Ladder 35+]] Phase VII
- [[Product Completion Checklist]]
- Cluster 26 gate baseline ([[Clusters/Cluster 26.0]])

## Out of scope

- Exhaustive multi-agent MCP matrix in gate e2e (CI cost).
- OAuth authorization-code app install flow.
- Cutting **`v2.0.0`** tag in the same PR as `v58.0.0` (maintainer action after review).
