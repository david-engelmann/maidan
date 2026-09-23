# Cluster 409 — Wave 4 #40: searchable, tag-honest release stream

> Post-gate hardening · target tag `v409.0.0` · umbrella issue #970

## Contract

- Treat `docs/Capabilities.md` as the searchable source-history stream and
  Git tags / GitHub Releases as the authority for published artifacts. A
  cluster-shaped heading alone must never be presented as proof that an image
  or binary was published.
- Keep the newest source record, changelog entry, contributor-guide version,
  and tag-matched GHCR examples mechanically aligned.
- Re-check that alignment against the actual pushed tag before a release job
  can build or publish anything.
- Put the release stream in the reader-facing mdBook navigation and explain
  how to move from a search result to immutable source, binaries, images, and
  verification evidence.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 409.1 | #979 | Release-record contract in CI and tag-time release gate |
| 409.2 | planned | Searchable release surface and published-vs-HEAD guidance |
| 409.close | close record | Ledgers, retrospective, and `v409.0.0` tag |

## Non-goals

- Backfilling missing historical tags or release artifacts.
- A second changelog, duplicated retrospectives, or generated marketing copy.
- A launch-week, brand, press, or hosted changelog product.
- Treating the mutable GHCR `latest` tag as reproducible deployment evidence.
