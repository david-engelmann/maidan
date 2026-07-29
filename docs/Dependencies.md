# Dependency currency & duplicate-version policy

How Maidan keeps dependencies current and bounds duplicate versions, and what's
blocked upstream. Enforced by `cargo deny check` in the `lint` CI job
(config: [`deny.toml`](../deny.toml)).

## Duplicate-version policy (Cluster 119)

`[bans] multiple-versions = "deny"` — a **new** duplicate major fails CI. Our
first-party crates therefore can't silently introduce one. The unavoidable
duplicates are explicit, reasoned exceptions in `deny.toml`:

- **`skip-tree`** quarantines whole vendored subtrees whose internal
  crypto/HTTP/TLS duplicates we don't control:
  - `aws-config`, `aws-sdk-s3` — the AWS SDK pins an older crypto/HTTP/TLS stack
    (`hmac` 0.12 **and** 0.13, `sha2`, `md-5`, `rustls` 0.21, `h2` 0.3,
    `hyper` 0.14).
  - `openidconnect` — v4 pins `base64` 0.21, `rsa`, and `oauth2`→`thiserror` 1.
  - `testcontainers` — dev-only (integration tests); `bollard` pulls its own
    HTTP/`base64` stack.
- **`skip`** lists cross-cutting ecosystem transitions pulled from many crates
  (not confined to one subtree): `getrandom`/`rand` 0.8→0.9, `hashbrown`
  0.15/0.16/0.17, `windows-sys` 0.48/0.52, `itertools`, `metrics-util`.

**Adding an accepted duplicate:** prefer `skip-tree` for a vendored root;
otherwise add a `skip` entry — always with a `reason`. If a bump makes an entry
unnecessary, `cargo deny check` flags it ("unnecessary skip"); remove it.

## Currency status of the named majors

| Crate | Our usage | Duplicate? | Why / when it clears |
|-------|-----------|-----------|----------------------|
| `thiserror` | **2.x** (workspace) | 1.x persists transitively | `metrics-exporter-prometheus`, `tungstenite`, `oauth2` (via openidconnect v4) still on 1.x. Our side is ready; resolves when they bump. |
| `base64` | **0.22** (our crates) | 0.21 from openidconnect v4 | Clears with **openidconnect v5** (see below). |
| `hmac` | 0.12 (server) | 0.12 **and** 0.13, both AWS-internal | Inside the AWS SDK crypto stack (`aws-sigv4`/`p256`/`hkdf`); not ours to collapse. |

## `anyhow` — RUSTSEC-2026-0190 (fixed by upgrade, Cluster 143)

**RUSTSEC-2026-0190** (unsoundness in `anyhow::Error::downcast_mut()` when the
error carries `.context()`) landed in the advisory DB and failed the required
`lint` job's `cargo-deny` advisories check for every PR. Fixed by bumping
`anyhow` **1.0.102 → 1.0.104** (`cargo update -p anyhow`; the fix is in
`>= 1.0.103`). Lockfile-only — no `Cargo.toml` change (our req is `1`). No
ignore added; this is a clean upgrade-away.

## openidconnect v5 — tracking item

openidconnect **v5 is not yet published** (latest is `4.0.1`). The v4 subtree is
the sole source of three accepted exceptions:

- `base64` 0.21 (duplicate vs our 0.22),
- the **`rsa` advisory `RUSTSEC-2023-0071`** (Marvin timing attack; no fixed
  `rsa` release) — used only for RS256 `id_token` signature verification,
- `oauth2`→`thiserror` 1.x.

**When v5 ships:** bump `openidconnect` to v5, then (1) remove the
`openidconnect` `skip-tree` entry, (2) re-run `cargo deny check` to confirm
`base64`/`rsa`/`thiserror`-1 drop, and (3) **remove the `RUSTSEC-2023-0071`
ignore** from `[advisories]`. Until then the ignore stands with the reasoning
above.

## Edition 2024 — evaluated, deferred (Track V/X)

The workspace **compiles cleanly** under edition 2024 (`cargo build --workspace`
on the pinned 1.91 toolchain). Adoption is **deferred** to a focused Track-V/X
migration PR because:

- `cargo clippy --all-targets -- -D warnings` surfaces `clippy::collapsible_if`
  (the let-chains lint) across several crates (`maidan-types`,
  `maidan-observability`, …) — mechanical but cross-cutting fixes.
- Edition 2024 changes `if let` temporary scope (temporaries drop at the end of
  the `if let`, not the enclosing block), which can shift drop/lock timing — so
  adoption needs the **full integration suite** to validate behavior, not just a
  clean build.

The migration is `cargo fix --edition` + the clippy fixes + a full e2e run.

## See also

- [`deny.toml`](../deny.toml) — the enforced policy.
- [Production.md](Production.md), [Operations.md](Operations.md).
