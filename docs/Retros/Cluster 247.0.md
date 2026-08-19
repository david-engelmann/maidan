# Cluster 247.0 retro — Maidan learns to send email (Arc I opens)

> Tag **`v247.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 11** — opens Arc I.

## What shipped

- The `lettre` SMTP dependency (rustls + tokio) + `0BSD` on the licence allow-list.
- A `mail.rs` module: `MailTransport` trait + `SmtpTransport` (lettre) + `SmtpConfig`
  from `MAIDAN_SMTP_*`. The first off-platform delivery transport — config-gated and
  unwired, so no behaviour change.

## Surprises / decisions

- **The dependency was the whole risk, and it came in clean.** Adding an external
  crate to a codebase with a strict `cargo deny` (advisories + bans-on-duplicate-major
  + licence allow-list) is where a "simple" feature bites. lettre with the rustls +
  tokio features introduced **no** new duplicate-major (bans ok) and **no** advisory
  (advisories ok) — the only gate it tripped was licences, and only because `0BSD`
  (BSD Zero Clause — fully permissive, public-domain-equivalent) wasn't yet in the
  allow-list. One line in `deny.toml` and it's green. Choosing the rustls features
  (matching the existing reqwest stack) is what kept the crypto subtree from
  duplicating.
- **Runtime-gate, don't cargo-feature.** The instinct for an optional dependency is a
  cargo feature, but that means `#[cfg(feature = "email")]` threading through the
  router/delivery code — exactly the module-split cfg ripple that has broken
  bootstrap-strip before. Making lettre a normal dep and gating on
  `SmtpConfig::from_env() -> Option` keeps every call site cfg-free; the cost is one
  always-compiled crate, and `--no-default-features` still builds. The right trade for
  this codebase.
- **Build without connecting = testable.** lettre pools SMTP connections lazily, so
  `SmtpTransport::from_config` validates the `from`/host without a network round-trip.
  That makes the config-build path and the bad-address rejection unit-testable in CI
  (no SMTP server); the actual send is a live-MTA / manual exercise.

## Capability table extension

| Change | Where |
|--------|-------|
| `lettre` dep + `0BSD` allow; `MailTransport` + `SmtpTransport` + `SmtpConfig` | `Cargo.toml`, `deny.toml`, `mail.rs`, `lib.rs` |

## Risks identified + still open

- None new. The dependency cleared `cargo deny`; the transport is inert without
  config.

## Forward look

Arc I continues: **248** a `member_emails` store (recipient addresses — members have
no email field today), then **249** the wiring — an email-delivery preference + the
router/worker calling `MailTransport::send` for opted-in members — plus the REST/MCP
to set your address + preference. Then scheduled digests, presence-aware routing
(needs durable `last_seen`), and the `/ui` notification center. Then Program D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 246.0]];
opens Arc I after Arc H (preferences + subscription) closed at 246.
