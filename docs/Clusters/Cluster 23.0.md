# Cluster 23.0 — Web UI product

Cluster 22.0 closed capability hardening at **`v22.0.0`**. The static UI under
`/ui` exists from Cluster H but lacks full product flows.

> **Goal:** Usable web UI for FSM transitions, search, MCP token UX, and thread
> detail.
>
> **Target tag:** `v23.0.0`.

## PRs

| #          | Title                                      | Issue |
|------------|--------------------------------------------|-------|
| kickoff    | `docs: Cluster 23.0 kickoff` (this doc)    | —     |
| 23.0.1+    | UI slices (FSM, search, tokens, thread) | TBD   |
| 23.0.retro | `docs(retro): Cluster 23.0 + v23.0.0`      | TBD   |

## Exit criteria

- Operator can drive core flows from `/ui` without raw HTTP.
- `v23.0.0` tagged after retro.

## References

- [[Clusters/Product Ladder 17-27]], [[Retros/Cluster 22.0]], [[Capability Map]].
