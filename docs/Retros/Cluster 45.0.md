# Cluster 45.0 retro — Admin console

> Tag **`v45.0.0`**.

## What shipped

- `/ui` v4 Admin tab: audit viewer, workspace purge with typed confirm, federation peer list/create/delete.
- Token panel: comma-separated capabilities, revoke by token ID.
- Session read proxies for audit and peers.

## What was deferred

- Token list/history UI (no list HTTP API yet).
- Session-cookie writes for purge/peers without bearer.

## Forward look

Cluster **46**: edit history and message UX affordances.
