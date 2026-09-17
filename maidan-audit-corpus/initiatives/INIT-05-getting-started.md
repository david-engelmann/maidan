# INIT-05 — Getting started

**Findings:** F-14 (P1), F-15 (P2), F-16 (P2), F-17 (P2), F-18 (P2), F-19 (P2), F-20 (P2)
**Research:** `research/R-06-quickstart-forensics.md`

## Problem statement

The first-run experience is fragmented across variants that disagree with each other:

1. **Three quickstart variants, three auth stories (F-14).** The README offers one-liner, Docker, and no-Docker paths; each bootstraps auth differently, none of the copied commands are CI-verified, `MAIDAN_BOOTSTRAP=1` semantics confuse, and the one-liner can't mint tokens at all.
2. **Hidden prerequisites (F-15).** The no-Docker flow never states the Rust toolchain requirement; the insecure variant's two-flag requirement is buried inside a collapsed `<details>` block.
3. **"One command" is three steps (F-16).** The Docker path requires manual token copy/paste between steps — fine as UX, but don't call it one command.
4. **`.env.example` exists but is undiscoverable (F-17).** The template is curated ("Copy to .env and fill in"), yet neither the README quickstart nor `docs/Production.md` references it — new operators never find it.
5. **Compose collisions undocumented (F-18).** `maidan-a` vs `maidan-server` both want port 8080 depending on profile; there is no ports matrix.
6. **App services lack healthchecks (F-19).** Infra services (postgres, minio) have `healthcheck:` blocks; the `maidan-*` app services don't, and `compose.quickstart.yaml` has none — so `depends_on: condition: service_healthy` can't order startup where it matters.
7. **Apple Silicon undocumented (F-20).** Support notes exist only in `docs/Pi.md`, which nobody reads on the way in.

## Why it matters to an automation-layer consumer

Getting started is the top of the adoption funnel for the exact audience Maidan wants: builders wiring agents to infrastructure. Every variant that fails on a clean machine is a builder who evaluates something else.

## Advisory recommendation

- Converge on **one decision-tree quickstart**: prerequisites up front, then a single recommended path, with variants clearly marked as alternatives. One auth/bootstrap story, told once.
- CI-verify every copied command (extract and run, or at minimum lint that referenced flags/files exist). **And verify by running, not by reading** (see the #905 retro below): any check that only reads the pin files cannot catch the drift class that matters.
- Ship a curated `.env.example` with the 10–15 variables a new operator actually needs, commented; link the full reference. *(Correction, second pass: the file already exists and is curated — the remaining work is linking it from the quickstart and `docs/Production.md`, and keeping it in sync as variables are added.)*
- Add a ports matrix and resolve or document the 8080 profile collision; add `healthcheck:` blocks to the `maidan-*` app services and the quickstart compose file so startup is ordered where it matters.
- State the Rust toolchain prerequisite and Apple Silicon status where the no-Docker/Docker paths begin.

### The #905 retro: what it teaches about verification

PR #905 (in CI, 2026-09-16) is the best recent evidence for *how* quickstart claims should be checked, and its retro is worth quoting in full because it generalizes:

- **"Verified by running, not by reading."** The version pin looked fine in the file; it took starting the stack and reading `/health` to see the binary was 88 clusters behind the source next to it.
- **"Two places held the same pin and only one was obvious."** The compose default *wins* over the Dockerfile ARG, so `docker compose config` would not have shown the drift either — the Dockerfile value was dead weight that looked authoritative.

Advisory consequences: (1) a CI job that boots the quickstart stack and asserts the `/health`-reported version matches the expected pin would have caught this on the first drifted release; (2) wherever two files can hold the same value, one of them should be generated from the other — #905's comment documents the manual recompute, but documentation of a manual step is not enforcement of it (see INIT-01).

## Open questions for the building agent

- Which quickstart variant is the *recommended* one? The docs currently present three co-equal options; picking a default simplifies everything downstream.
- Is `MAIDAN_BOOTSTRAP=1` a permanent concept or scaffolding that should be replaced by a real first-run flow?
- How much of the 84-variable surface is actually user-facing vs internal? The `.env.example` curation depends on that split.

## Signals of resolution

- A clean-machine run of the documented quickstart works verbatim and is covered by CI.
- `.env.example` is linked from the quickstart and `docs/Production.md`, and stays in sync with the variable surface; ports matrix exists; app services and quickstart compose have healthchecks.
- Prerequisites (Rust toolchain, Apple Silicon status) are stated before step one.
