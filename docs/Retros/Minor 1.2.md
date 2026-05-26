# Minor 1.2 retro — Search + embeddings

> Closing wave for optional minor **`v1.2.0`** · [[Post-1.0]] ladder 1.2.1–1.2.3.

After post-1.0 tracks closed docs/ops/perf work, the next user-visible gap
was search ergonomics: pluggable embeddings for future models, faceted
lexical filters, and Postgres web-style query operators — without breaking
the stable HTTP/MCP search contract.

## What shipped

| PR   | Scope |
|------|-------|
| #122 | `EmbeddingProvider` trait, `HashV1Provider`, `MAIDAN_EMBEDDING_PROVIDER`. |
| #123 | Facets on `GET …/search` and MCP (`author`, `channel`, `kind`). |
| #124 | Postgres `websearch_to_tsquery` when `q` uses `"phrase"`, `-negation`, or `or`. |

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| `v1.3.0`  | Real ML embedding provider + semantic HTTP surface | Needs provider + API design.             |
| `v1.3.0+` | Semantic search facets                              | Depends on semantic query path.          |
| Track T   | Coverage minimum % gate                           | Optional; artifact exists (T.3).         |
| Track V   | Automated cosign on release                       | Manual per [[Operations]] until keyless. |
| Open Work | SQLite `sqlite-vec` semantic search               | Extension maturity.                      |

## Surprises

- **Phrase queries** — `"systems programming"` returned zero hits with
  stemming; `"ferris the unofficial"` was a reliable integration anchor.
- **Negation tests** — `rust -deployment` does not shrink a `rust` result
  set when no rust hit contains `deployment`; use `rust -tokio` instead.
- **SQLite facet SQL** — duplicate binds for `(? IS NULL OR col = ?)` per
  sqlx positional rules.

## Decisions

- **`kind` facet = author `MemberKind`** — not message or artifact kind.
- **Websearch is opt-in by syntax** — plain `q=rust tokio` keeps
  `plainto_tsquery` behavior; operators flip to `websearch_to_tsquery`.
- **Embedding default unchanged** — `hash-v1` remains the only built-in
  provider; env selects among registered providers later.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Pluggable embedding provider + `MAIDAN_EMBEDDING_PROVIDER` | `v1.2.0`        |
| Search facets (`author`, `channel`, `kind`) on HTTP/MCP | `v1.2.0`           |
| Postgres websearch operators in `q`                       | `v1.2.0`           |

## Risks identified + still open

- **`hash-v1` is not semantic** — operators may expect vector quality;
  document until a real provider ships.
- **Facet + operator parity** — SQLite ignores websearch operators; semantic
  search ignores facets today.
- **Bootstrap routes** remain unauthenticated (unchanged).

## Forward look

Optional **`v1.3.0`** — real embeddings + semantic search UX per
[[Post-1.0]]. Ad-hoc backlog in [[Open Work]] (OAuth, coverage gate, S3
multipart).

## Acknowledgements

Solo minor. Three PRs, one retro, tag `v1.2.0`.
