# Cluster 292.0 retro — Architecture docs currency + split

> Tag **`v292.0.0`**. Phase XXIV (post-gate hardening). Launch-readiness P1. Docs. No new
> gate tag.

## What shipped

Split the 549-line, version-history-interleaved `docs/Architecture.md` into:

- **`docs/Architecture.md`** — a clean, **version-neutral, current** conceptual overview:
  one-paragraph summary, system + component diagrams, crates, data layering,
  backends, the API surface, and present-tense **subsystem** sections (artifacts, the
  thread/task layer, search, auth & RBAC, realtime & delivery, notifications, federation &
  A2A, scale & ops). No `vX.0.0` baselines and no internal "Cluster N" vocabulary on the
  first user-facing page.
- **`docs/Architecture-history.md`** — the release-by-release record (the `at vX.0.0`
  sections + the substrate snapshot + the scale-out ladder), wired into the book under
  Design, linked from Architecture.md.

## Surprises / decisions

- **It wasn't just interleaved — it was stale.** The conceptual sections effectively
  stopped around `v104`/ladder-120: the agentic task layer (DAG/skills/scheduling/results),
  the notifications & reach subsystem, the three-transport A2A binding, the LSN causal
  read-replica, and the per-channel/thread RBAC arc were **not described at all** despite a
  `v273` baseline header. So the "split" required "currency" — the conceptual doc was
  rewritten to describe the system as it actually is today, with the historical accretion
  preserved (condensed) in the history file.
- **Anchors moved cleanly.** Version-dated sections (and their `#anchors`) moved to the
  history doc; mdbook linkcheck stayed green, so nothing cross-linked the moved anchors.
- **History condensed, not lost.** `Architecture-history.md` keeps the substance;
  `Capabilities.md` + `CHANGELOG.md` remain the authoritative per-release lists (both
  linked).

## Capability table extension

Docs only — no capability change.

## Risks identified + still open

- The history doc condenses some original sections rather than reproducing them verbatim;
  the authoritative detail is in the per-cluster retros + Capabilities/CHANGELOG.

## Forward look

Launch-readiness polish continues: **293** GitHub repo metadata (topics, homepage, issue
templates), then the **SDK arc** (294+, TS→Python→Go→Rust to a usable 0.1.0).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 291.0]].
