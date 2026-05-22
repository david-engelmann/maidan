# Capabilities

A running list of what Maidan can do, by release. Each cluster's retro
PR prepends a new section so the latest is always at the top.

## v1.0.0 (target)

Populated when Cluster H lands.

## v0.0.1 — Cluster A complete

| Capability                                              | Surface                |
|---------------------------------------------------------|------------------------|
| Persistent core schema (Postgres + SQLite)              | `maidan-store`         |
| Dialect detection from `DATABASE_URL` prefix            | `maidan-store::Dialect`|
| Cross-dialect parity test                               | `maidan-store/tests`   |
| Content-addressed artifact body store (LocalFs)         | `maidan-artifacts`     |
| Atomic, dedup-safe artifact writes (50-task concurrent) | `maidan-artifacts`     |
| `/health` endpoint reporting DB + storage status        | `maidan-server`        |
| `docker compose up` brings up Postgres + MinIO + server | `compose.yaml`         |
| Hot-reload dev compose stack                            | `compose.dev.yaml`     |
| Kustomize base + dev + prod overlays                    | `k8s/`                 |
| testcontainers-backed integration suite                 | `maidan-store/tests`   |
| Obsidian docs vault                                     | `docs/`                |
