# Capabilities

A running list of what Maidan can do, by release. Each cluster's retro
PR prepends a new section so the latest is always at the top.

## v1.0.0 (target)

Populated when Cluster H lands.

## v0.0.1 — Cluster A complete (planned)

| Capability                                              | Surface              |
|---------------------------------------------------------|----------------------|
| Persistent core schema (Postgres + SQLite)              | `maidan-store`       |
| Content-addressed artifact body store (LocalFs)         | `maidan-artifacts`   |
| `/health` endpoint reporting DB + storage status        | `maidan-server`      |
| `docker compose up` brings up Postgres + MinIO + server | `compose.yaml`       |
| testcontainers-backed integration suite                 | `maidan-store/tests` |
