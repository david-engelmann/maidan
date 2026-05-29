# Cluster 49.0 — Agent context export

**Theme:** One-shot thread context for agent prompt packing.

## Problem

Agents need messages, references, artifacts, and FSM state together when
building prompts. Today that requires multiple HTTP/MCP calls.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `list_thread_transitions` |
| HTTP | `GET /threads/:id/context` |
| Assembly | Messages + refs + metadata-linked artifacts + FSM history |

## Tag

`v49.0.0`

See [[Clusters/Product Ladder 35+]] Phase IV.
