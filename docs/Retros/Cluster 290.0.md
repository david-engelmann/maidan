# Cluster 290.0 retro — a2a-interop harness hotfix

> Tag **`v290.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

A one-file fix to `scripts/a2a-interop.sh` so the report-only `a2a interop` CI job
(Cluster 289) is reliably green. The harness used `cargo run` (compile **then** boot) with
a 120 s health-wait; on a cold CI cache the compile alone outlasted the wait, so the
conformance client ran against a server that wasn't up yet and failed with
`httpx.ConnectError: Connection refused`. (The job is `continue-on-error`, so it never
blocked a merge — but a permanently-red report-only job is misleading noise.)

Now the harness:
1. `cargo build`s the server first (blocking — the slow part is done before any wait),
2. runs the built `./target/debug/maidan-server` in the background,
3. waits up to 60 s for `/health` and **fails fast with a clear message** if it never comes
   up (instead of silently running the client against nothing).

Verified locally end-to-end (build → boot → all conformance checks pass → exit 0).

## Surprises / decisions

- **Live-verified locally ≠ CI-safe.** The 289 harness passed locally because the binary
  was already built; the cold-cache CI compile-time was the gap. Splitting build from
  boot is the right shape for any "boot a source server + poke it" job.
- **Kept it report-only.** The fix makes the job green, but it stays `continue-on-error`
  and non-required — the A2A binding behavior is gated by the required Rust e2e tests; this
  job is the external-client cross-check.

## Risks identified + still open

- None new. The A2A arc's logged follow-ups (Open Work) are unchanged.

## Forward look

The A2A v1.0 compliance arc (282–289) remains complete; this only stabilizes its CI job.
Remaining launch-readiness items are the smaller polish tracks (Architecture split, GitHub
metadata) in Open Work.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows
[[Retros/Cluster 289.0]].
