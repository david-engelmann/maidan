# Cluster 313.0 retro — default-secure quickstart (launch hardening F4)

> Tag **`v313.0.0`**. Phase XXIV (post-gate hardening). Launch-prep (Pre-Public
> Hardening **F4** / Launch **L1**). No new gate tag.

## What shipped

The quickstart happy path no longer teaches `AUTH_DISABLED` — it mints a real bearer
token. Launch.md L1 is blunt about why: *"one `AUTH_DISABLED` screenshot kills the
launch."*

- **`compose.quickstart.yaml` runs auth ON** — removed `AUTH_DISABLED` /
  `MAIDAN_ALLOW_INSECURE_NO_AUTH`, added a dev `MAIDAN_SESSION_SECRET` and
  `MAIDAN_BOOTSTRAP=1` (so the demo can still seed its two agent members over the
  bootstrap route while *content* operations require the token).
- **`maidan init` is the happy path.** The quickstart image
  (`docker/Dockerfile.quickstart`) already bundles the `maidan` CLI; bumped its pinned,
  SHA-256-verified release from **`v277.0.0` → `v312.0.0`** because `maidan init` only
  landed in `v279` (Cluster 279). The README now shows: `up` → `maidan init --workspace
  demo` → run the demo with the printed token + workspace id.
- **`scripts/quickstart-two-agents.sh` is auth-aware.** `MAIDAN_TOKEN` set → bearer on
  every call, reuse the `maidan init` workspace (`MAIDAN_WORKSPACE`); unset → the
  legacy insecure path. Members are seeded over the bootstrap route (open in both modes).
- **`compose.quickstart.insecure.yaml`** (new) — a clearly-labelled local-only override
  that layers `AUTH_DISABLED` back on for "explore without a token", demoted to a README
  `<details>` appendix.
- **README + Integration.md** rewritten: the happy path is token-based end to end (the
  60-second REST snippet carries `Authorization: Bearer`), and Integration.md's seed
  section leads with `maidan init` instead of a circular "mint needs `token:admin`".
- **CI** validates both compose files (`config -q` on the base and the base+override
  merge) plus `bash -n` on the script.

## Surprises / decisions

- **The `full` compose profile was already auth-on.** The grep that flagged
  `AUTH_DISABLED` in `compose.yaml` matched the federation/scale/otlp *smoke* profiles,
  not the quickstart — the real F4 gap was the **quickstart** README + `compose.quickstart.yaml`.
- **Version bump was forced, not cosmetic.** `maidan init` didn't exist at the pinned
  `v277`; the token happy path is impossible without bumping the image. Re-pinned the two
  release-tarball SHA-256s (downloaded + hashed the `v312.0.0` assets).
- **No authenticated member-create exists** — member creation is bootstrap-only. So the
  demo keeps `MAIDAN_BOOTSTRAP=1`; auth is enforced where it matters (channels/threads/
  messages need the token). The README says to unset bootstrap once a real deployment has
  its admin.
- **bash 3.2 nounset bug caught in validation.** An empty `"${AUTH[@]}"` under `set -u`
  aborts on macOS bash 3.2 (the insecure path); fixed with the `${AUTH[@]+"${AUTH[@]}"}`
  guard. Only surfaced because I ran the script for real, not just `bash -n`.

## Validation

Both paths run end-to-end against a source-built server (not just file-lint):
token mode (auth on + `maidan init` + bearer) → exit 0, two messages posted + read back;
insecure mode (`AUTH_DISABLED`) → exit 0; an unauthenticated content POST → `401`. Both
compose files pass `docker compose config -q`; script passes `bash -n`.

## Risks / follow-ups

- The quickstart image pins `v312.0.0`; each future bump re-pins two SHAs (documented in
  the Dockerfile).
- Remaining launch-prep (**Cluster 314**): L3 human release-notes template, L4 honest
  claims sheet, L6 SECURITY/CONTRIBUTING language, L5 verify the last tag's cosign. The
  **public launch itself stays gated on the maintainer's explicit go** (Launch.md).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Closes Pre-Public Hardening
**F4** / Launch **L1**; the launch runbook is [[Launch]].
