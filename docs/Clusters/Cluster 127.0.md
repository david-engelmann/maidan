# Cluster 127.0 — Backlog reconciliation

**Theme:** Reconcile the backlog docs (`Remaining Work.md`, `Open Work.md`)
against the **actual code** at v126. This session repeatedly found "open"
backlog items that had already shipped (OTLP, durable reindex, the dead promtool
branch); a systematic verification pass corrects the record so the backlog is
trustworthy again.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v127.0.0`**, no new
gate tag. Docs-only.

**Predecessor:** all of 102–126.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Verify** | Parallel code-verification of every claimed-open backlog item (§1/§2/§3/§4/§7 + Open Work). |
| **Correct** | Strike the ~11 phantom (already-shipped) entries with the shipping cluster + file evidence; fix the stale `Open Work` tail (it still claimed "latest tag v76 / active cluster 78"). |
| **Classify** | Tag the Slack-parity §4 gaps as product/UI (complete backends) vs out-of-scope vs backend-tractable. |

## Phantom gaps corrected (already shipped)

group DMs (97), presence/typing (103), per-model embedding tables (86/0020),
`sqlite-vec` + CI (85), schema-parity tests, cosign signing (release.yml),
bootstrap compile-time strip (91), SQLite delivery cursor (implemented),
Helm prod profiles (88), context workspace thread cursor, Web UI tabs + WS tail,
OTLP export/dashboards/e2e (89/90/123), OpenAPI capability map (121).

## Genuinely-open items that remain

- Unify webhook + automation delivery tables (value debatable — they work fine separate).
- Global cross-workspace admin audit query API (data exists; expose it).
- mcp-stdio dedicated embedded-indexer mode (niche, low value).
- Full MCP streamable spec-complete bidirectional mux (subset shipped in 73).
- Plus the §4 product/UI features (need product decisions) and standing risks.

## Non-goals

- Implementing any of the open items — this is a reconciliation, not a feature
  cluster (those are sequenced as Clusters 128–132).

## PR ladder (actual)

| # | Title |
|---|--------|
| 127.0.1 + retro | `docs(retro): Cluster 127.0 — backlog reconciliation + v127.0.0 tag prep` (single docs PR) |

## Exit criteria

- `Remaining Work.md` + `Open Work.md` match the code at v126 — **met**.
- `v127.0.0` tagged.

## References

- [[Retros/Cluster 127.0]], [[Remaining Work]], [[Open Work]]
