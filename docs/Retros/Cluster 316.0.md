# Cluster 316.0 retro — honesty scrub + no-clone image

> Tag **`v316.0.0`**. Phase XXIV (post-gate hardening). Second cluster of the 2026-08-28
> research-sweep program. No new gate tag.

## What shipped

The docs honesty scrub (correct every verified lie a stranger or protocol reviewer hits at
`v315`), plus an honest prebuilt-image path — reshaped by a smoke that caught the planned
one from being impossible.

- **Claims.md A2A correction** — the sheet built to be honest overclaimed "full A2A v1.0
  across three bindings." gRPC exposes only `get_task`/`cancel_task`/`list_tasks` — **no
  `SendMessage`**. Softened to "JSON-RPC + REST complete; gRPC = task read/cancel/list."
- **Two more won't-boot commands fixed** (the class 314/315 kept finding):
  `book/src/introduction.md`'s published `cargo run` had no `MAIDAN_SESSION_SECRET`
  (auth-on → won't boot), and `Pi.md`'s `docker run -e AUTH_DISABLED=1` was missing the
  `MAIDAN_ALLOW_INSECURE_NO_AUTH` ack (fail-closed → won't boot) and used `:latest`. Both
  fixed + pinned; Pi.md's native path recast to `maidan init` (auth on).
- **Comment/doc lies** — `mail.rs` "Not wired" (wired since 249; also cleared the
  Pre-Public-Hardening A6/K1 P0), `mcp/server.rs` "default stays 2024-11-05" (const is 2026),
  `Framework Integrations.md` 2024→2026, `Threat-Model.md` seed → `maidan init`, `sdk/README`
  + `Clients.md`/`Client Testing.md` banners (SDKs published at 0.1.0, MCP 2026, not
  name-holds), `Promotion.md` state banner (projectors/mail/SDK shipped, topics set, hero no
  longer cargo+AUTH_DISABLED), README "experimental A2A bridge" → "A2A v1.0 (JSON-RPC+REST;
  gRPC partial)", `AGENTS.md`/`Integration.md` MCP-2024/A2A-subset, `CLAUDE.md` latest-tag
  v273→v315 (+ "Open Work is canonical"), `SECURITY.md` cosign example → `<tag>`.
- **`v300.0.0` release** was a stuck Draft (all neighbors published) → **published**.
- **No `v311` tag** (311's code is in `v312`) — documented, tag NOT cut (correct).

## The finding: the "no-clone one-command eval" the doc planned is impossible

The plan (Undeniable Final §3.1) was a README `docker run ghcr.io/…:v314` then
`exec maidan init`. **Smoked before writing it, and the smoke killed it:** the published
`ghcr.io/david-engelmann/maidan-server:v315.0.0` **boots with auth on** (`/health` ok,
multi-arch, anonymously pullable) — but it's a **distroless single binary**: no shell, and
it bundles **only `maidan-server`, not the `maidan` CLI** (the prod Dockerfile builds
`--bin maidan-server` only). `POST /workspaces` → 401. So `docker run` gives a healthy
server with **no way to get a token into it** — the "then `exec maidan init`" step can't
run (no CLI, no shell).

So I shipped the **honest** version — a README "Prebuilt image (no clone)" note: images are
signed + multi-arch, deploy without cloning, seed via `maidan init` run against your DB (a
release binary or a one-shot job), verify cosign — **without a fake one-command-with-token**.
The *true* one-command no-clone eval (token flow bundled) needs the **quickstart image**
(both binaries + a shell) **published to GHCR** — real infra, deferred to its own cluster.

**Lesson (again):** smoke the artifact before advertising it. 314 caught a broken README
one-liner via the claims sheet; 316 caught a broken README *docker* one-liner via an actual
image pull. The pattern holds — verify reachability/runnability, don't infer it.

## Residual (noted, low blast radius)

The dense planning docs (`Clients.md`, `Client Testing.md`, `Promotion.md`) still have inline
0.0.1/2024/AUTH_DISABLED references beyond the corrected top banners; the strategy pack
(Handoff/Path/Expansion Bets/Launch/Adoption) stays a frozen 2026-08-25 snapshot. All
maintainer-facing; canonical status is Open Work. A full inline sweep is optional.

## Test evidence

Comment-only `.rs` edits compile (`cargo check -p maidan-server -p maidan-mcp`); mdbook
linkcheck green across the scrubbed published docs; the published image smoke ran live
(pull + `docker run` + `/health` ok + CLI-absence + `POST /workspaces` 401).

## Forward look

Next: **317** Bet 2 MCP snippet pack, **318** token-pack evidence, then the fidelity +
context flagship arc. A follow-up cluster: publish the quickstart image to GHCR for a true
no-clone eval. Launch stays gated on the maintainer's go.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the 2026-08-28 sweep
folded into [[Open Work]].
