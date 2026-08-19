# Cluster 247.0 — email/SMTP transport foundation (opens Arc I)

> **Program C (notifications & reach), part 11** — opens **Arc I (transport + reach)**.
> Phase XXIV post-gate hardening. Tag **`v247.0.0`**. No new gate tag.

## Goal

Open Arc I with the first off-platform delivery transport: a `MailTransport` trait +
a `lettre`-backed SMTP transport + config, so a notification can (in a later cluster)
be delivered by email. Config-gated and **unwired** — no behaviour change until a
recipient opts in.

## Scope

| Change | Where |
|--------|-------|
| `lettre` dependency (rustls + tokio, no openssl) + `0BSD` added to the licence allow-list | `crates/maidan-server/Cargo.toml`, `deny.toml` |
| `MailTransport` trait + `SmtpTransport` (lettre) + `SmtpConfig::from_env` (`MAIDAN_SMTP_*`) | `mail.rs`, `lib.rs` |

## Design decisions

- **`lettre`, rustls + tokio, no openssl.** Matches the existing `reqwest`/`rustls`
  TLS stack (`tokio1` + `tokio1-rustls-tls` + `smtp-transport` + `builder`), so no new
  crypto backend enters the tree. `cargo deny` passes: **advisories/bans ok** (no new
  duplicate-major), and **licences** needed only `0BSD` added to the allow-list —
  lettre is BSD-Zero-Clause (fully permissive, public-domain-equivalent).
- **A normal dependency, runtime-gated — not a cargo feature.** `SmtpConfig::from_env`
  returns `None` unless `MAIDAN_SMTP_HOST` + `MAIDAN_SMTP_FROM` are set, so a default
  deployment builds no mailer and sends nothing. Runtime-gating (vs a cargo feature)
  keeps the router/delivery code cfg-free — no `#[cfg]` ripple through the codebase
  (the memory `maidan-module-split-ripples` lesson) — at the cost of always compiling
  lettre (one crate). Verified `--no-default-features` still compiles (bootstrap-strip).
- **A trait, for the next transport.** `MailTransport::send(to, subject, body)` is the
  seam SMS/push slot into later; today the one impl is `SmtpTransport`.
- **Build ≠ connect.** `SmtpTransport::from_config` only validates the `from` address
  and host (lettre pools connections lazily on `send`), so it's unit-testable without
  an SMTP server — the config-build + bad-address-rejection paths are tested; a real
  send is exercised against a live MTA (a later cluster / manual).

## Non-goals / deferred

- **Recipient addresses** — members have no email field yet; a `member_emails` store
  (Cluster 248) is the next prerequisite.
- **Wiring** — delivering a member's notifications by email (an email-delivery
  preference + the router/worker calling `MailTransport::send`) is Cluster 249.
- Implicit-TLS-only SMTP servers (only STARTTLS + plaintext-relay modeled); digests,
  presence-aware routing, `/ui` center (rest of Arc I).

## Risks

- The new dependency was the headline risk; `cargo deny check` (all four sections)
  passes with the single `0BSD` allow. Adding `lettre` is confined to `Cargo.toml` +
  `deny.toml` + the new `mail.rs`; nothing else changes.
