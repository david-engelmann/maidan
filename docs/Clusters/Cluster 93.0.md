# Cluster 93.0 — /ui live events

**Theme:** /ui live events.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XVII · tag **`v93.0.0`**.

**Predecessor:** Cluster **92** channels UI.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XVII row for cluster **93**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | `/ui` WS subscribe panel with filter presets + reconnect. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Full event debugger.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 93.0.1 feat(server): ui live event tail |
| 93.0.2 test(server): ui_ws_tail_e2e |
| 93.0.retro docs(retro): Cluster 93.0 + v93.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v93.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 92.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]
