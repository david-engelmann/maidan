# Cluster 409 retro — a searchable, tag-honest release stream

> Post-gate hardening · `v409.0.0` · umbrella #970 · PRs #979/#982 + close record

## Outcome

Wave 4 row #40 is closed. Maidan now has one reader-facing release stream that
answers two different questions without conflating them: what exists in source,
and what was actually published as an immutable tag with artifacts. The record
is searchable by capability or exact version, and both ordinary CI and the
tag-triggered release workflow derive their verdict from the repository's real
tag set.

| Slice | Evidence | Result |
|-------|----------|--------|
| 409.1 | #979; `scripts/check-release-records.sh`; ordinary and release CI | The newest capability record, changelog, contributor-guide version, and tag-matched README images must agree. Publication stops before binaries or images when the pushed tag disagrees. |
| 409.2 | #982; mdBook build and presentation contract; full agent contract | 328 release headings link to real immutable tags, 79 never-tagged milestones are explicitly source-only, and 12 folded tags remain exact-searchable. Reader docs explain published artifacts versus newer `main` source. |

## Decisions

- Git tags are the authority for whether a release exists. A cluster-shaped
  source record is useful history, but it is not evidence that a binary or
  container was published.
- Missing historical tags stay missing. The stream labels those records rather
  than inventing tags and artifacts after the fact.
- A close PR may prepare exactly one newest, fully aligned release link before
  its tag exists. Tag-triggered validation receives no such exception; the tag
  must exist and match before publication can proceed.
- `docs/Capabilities.md` is the concise public release stream;
  `CHANGELOG.md` remains the detailed change log, and cluster plans/retros keep
  implementation rationale. The three records are complementary, not copies.

## What surprised us

- The existing ledger contained 79 cluster records with no corresponding Git
  tag and 12 real tags folded into adjacent records. Heading shape alone had
  made both groups look less precise than the repository history actually is.
- Validating against tags requires full checkout history. A correct local
  checker would have become a false CI failure under the default shallow
  `actions/checkout` behavior without explicitly fetching all tags.
- The full agent contract exposed an unrelated blind spot in the generated HTTP
  capability matrix: the new advice route received `{}`, so request extraction
  returned 400 before authorization could return 403. PR #981 supplied the
  smallest schema-valid fixture and restored exhaustive denial coverage.
- Closing a release record and cutting its tag form a bootstrap cycle. Making
  that exception narrow and asymmetric—ordinary CI only, newest record only—
  preserves a fail-closed tag-time gate.

## Residual risk and follow-up

- GitHub availability still controls release links and artifact download. The
  immutable tag remains independently cloneable, and release verification uses
  checksums/signatures, but this cluster does not mirror release artifacts.
- Historical source-only records describe merged code, not a supported binary.
  Consumers who need one must build the cited source commit themselves.
- The next planned Wave 4 row is #41, the PayerStamp and denial-audit lane. It
  changes durable event/audit data and should start from its own contract rather
  than being folded into release documentation.

## Release ledger

| Item | Value |
|------|-------|
| Tag | `v409.0.0` |
| Roadmap | Wave 4 row #40 closed |
| Database compatibility | No migration |
| API compatibility | No runtime API change |
| Runtime behavior | Unchanged; documentation and release publication contract only |
| New release gates | Tag-derived source-record alignment in ordinary CI and before release publication |
