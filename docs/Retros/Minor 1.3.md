# Minor 1.3 retro — Semantic search UX

> Closing wave for optional minor **`v1.3.0`** · [[Post-1.0]] ladder 1.3.1–1.3.3.

This minor turns semantic search from internal plumbing into an operator-visible
surface: HTTP/MCP query mode, real remote embeddings configuration, and health
visibility when embedding generation/indexing fails.

## What shipped

| PR   | Scope |
|------|-------|
| #126 | `GET /workspaces/:wid/search?mode=semantic` (Postgres) + MCP `search_messages.mode`. |
| #127 | OpenAI-compatible embedding provider config + `/health/ready` indexer embedding failure surfacing. |

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| `v1.4.0`  | `MAIDAN_BOOTSTRAP=1` one-shot seed gate           | Auth-hardening focus.                    |
| `v1.4.0+` | OAuth/OIDC integration                            | Larger auth/session design surface.      |
| Open Work | Semantic facets (`mode=semantic` + author/channel/kind) | Needs ranking/filter semantics agreement. |
| Open Work | Per-model embedding tables / mixed dimensions     | Schema + query contract work.            |
| Track T   | Coverage minimum % gate                           | Optional; artifact exists (T.3).         |

## Surprises

- **HTTP mode parity reached MCP first** — plumbing to share provider into MCP
  surfaced quickly once server state owned the provider.
- **Health signal wiring** — the simplest reliable path was a shared
  `Arc<RwLock<Option<String>>>` updated by the embedding handler.
- **Remote provider ergonomics** — OpenAI-compatible payload shape is stable,
  but retry/backoff policy still needs explicit design.

## Decisions

- **Semantic mode reuses `q`** — no separate route yet; `mode` selects lexical
  vs semantic behavior.
- **Facets stay lexical-only for now** — semantic mode rejects facet params
  until query semantics are defined.
- **Provider errors are surfaced** — HTTP/MCP semantic paths fail fast instead
  of returning ambiguous empty results.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| HTTP semantic search mode (`GET …/search?mode=semantic`) | `v1.3.0`         |
| MCP semantic search mode (`search_messages.mode`)         | `v1.3.0`         |
| OpenAI-compatible remote embedding provider config        | `v1.3.0`         |
| `/health/ready` embedding indexer failure visibility      | `v1.3.0`         |

## Risks identified + still open

- **Remote embedding latency/cost** — no adaptive batching/backoff yet.
- **SQLite semantic gap** — semantic mode remains Postgres-only.
- **At-most-once event delivery** remains (unchanged standing risk).

## Forward look

Optional **`v1.4.0`** auth hardening: bootstrap gating and OAuth/OIDC planning.
Remaining non-auth backlog continues in [[Open Work]].

## Acknowledgements

Solo minor. Two PRs, one retro, tag `v1.3.0`.
