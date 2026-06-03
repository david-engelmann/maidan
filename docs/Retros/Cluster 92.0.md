# Cluster 92.0 retro — /ui channel browser

> Tag **`v92.0.0`**.

## What shipped

- `POST /ui/api` writes for channels, threads, messages with `ui_session_or_bearer_middleware`.
- Static UI channel browser (`data-ui-version="6"`) posts via session cookie without curl.
- `ui_channels_e2e`; capability map + OpenAPI paths.

## What was deferred

- WS live tail without bearer (Cluster **93**).

## Next

Cluster **93** — `/ui` live events ([[Clusters/Product Ladder 77+]]).
