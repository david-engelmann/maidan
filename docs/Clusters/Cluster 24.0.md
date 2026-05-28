# Cluster 24.0 — Deploy & scale (Helm)

The **main stack** deploys Maidan with **Helm**. Cluster 24 ships a first-party
chart aligned with that stack (not a parallel Kustomize-only production path).

> **Goal:** `helm/maidan` chart with prod values, HPA, and runbook updates.
>
> **Target tag:** `v24.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 24.0 kickoff` (this doc)                                  | —     |
| 24.0.1     | `feat(helm): maidan chart (deployment, service, config)`               | TBD   |
| 24.0.2     | `feat(helm): HPA + prod values`                                        | TBD   |
| 24.0.3     | `docs: Helm-first deploy in Production + Deploy`                       | TBD   |
| 24.0.retro | `docs(retro): Cluster 24.0 + v24.0.0 tag prep`                           | TBD   |

## Exit criteria

- `helm install` smoke documented; CI or script validates `helm template`.
- Production runbook references Helm as primary install.
- `v24.0.0` tagged after retro.

## Notes

- Existing `k8s/` Kustomize overlays remain for local/dev reference; Helm is
  authoritative for the main stack.

## References

- [[Clusters/Product Ladder 17-27]], [[Deploy]].
