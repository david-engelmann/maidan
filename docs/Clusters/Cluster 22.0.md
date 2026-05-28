# Cluster 22.0 — Capabilities hardening

Cluster 21.0 closed A2A protocol ingress at **`v21.0.0`**. Auth tokens carry
capability strings, but not every route has negative tests proving enforcement.

> **Goal:** Every HTTP/MCP/WS path checks the right capability; CI includes
> denial cases.
>
> **Target tag:** `v22.0.0`.

## PRs

| #          | Title                                              | Issue |
|------------|----------------------------------------------------|-------|
| kickoff    | `docs: Cluster 22.0 kickoff` (this doc)            | —     |
| 22.0.1     | `test(server): capability denial e2e matrix`       | TBD   |
| 22.0.2     | `fix(server): close capability gaps on routes`     | TBD   |
| 22.0.retro | `docs(retro): Cluster 22.0 + v22.0.0 tag prep`      | TBD   |

## Exit criteria

- Documented capability map for HTTP + MCP + WS.
- E2E tests prove 403 without required capability.
- `v22.0.0` tagged after retro.

## References

- [[Clusters/Product Ladder 17-27]], [[Retros/Cluster 21.0]].
