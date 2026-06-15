# Cluster 111.0 — `maidan-auth` test suite

**Theme:** Bring the authn/authz crate up to a coverage bar matching its blast radius — capability checks, constant-time bearer comparison, and at-rest peer-secret crypto.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXI · tag **`v111.0.0`** · **opens the phase**.

**Predecessor:** Auth + capabilities from [[Clusters/Cluster 22.0]]; federation peer secrets from the A2A track.

---

## Problem

`maidan-auth` is the chokepoint every authenticated request passes through — it owns the capability vocabulary, the constant-time hash compare that gates bearer acceptance, and the ChaCha20-Poly1305 encryption of federation peer secrets at rest. Yet it carried only **five inline `#[cfg(test)]` unit tests** and **no `tests/` directory**: `context.rs` (the `AuthContext` authorization matrix) and `resolve.rs` (bearer → context resolution) had **zero** coverage, and the AEAD layer never asserted that *tampered* ciphertext is rejected. For the first cluster of a correctness-and-coverage phase, this is the highest-leverage gap to close.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Capabilities** | Matrix over `is_known` / `validate_list` / `validate_subset` / `default_minted` and the `AuthContext` constructors (token / app-token / session / bypass): grant/deny, `require_capability`, cross-workspace `ensure_workspace`. |
| **Crypto** | Peer-secret AEAD round-trip, **tamper detection** (ciphertext body + nonce), truncation / non-base64 rejection, and the `FEDERATION_ENCRYPTION_KEY` parse matrix. |
| **Lifecycle** | Store-backed `resolve_bearer` / `resolve_peer_bearer`: capability propagation, forged-secret rejection, post-revocation and post-expiry failure. |
| **Constant-time** | `hashes_equal` correctness incl. the length-mismatch guard (must not panic / index past the stored digest). |

## Non-goals

- Changing any `src/` behavior — this is a tests-only cluster.
- Re-proving `maidan-store`'s token/peer row CRUD (already covered in `maidan-store/tests/api_tokens.rs` / `federation_peers.rs`).
- End-to-end app-token resolution through HTTP (covered by the server e2e suite).

## PR ladder (actual)

| # | Title |
|---|--------|
| 111.0.1 | `test(auth): maidan-auth test suite — capabilities, context, AEAD, bearer lifecycle` (#308) |
| 111.0.retro | `docs(retro): Cluster 111.0 + v111.0.0 tag prep` |

## Exit criteria

- `maidan-auth/tests/`: capability resolution matrix, token lifecycle/revocation, peer-secret AEAD round-trip + tamper, constant-time paths — **met** (26 integration tests).
- `v111.0.0` tagged after retro.

## Ordering & risks

- **Independent of [[Clusters/Cluster 112.0]]** — both are pure-logic crates and could start immediately; this one shipped first.
- **Risk — env-var test contention:** the `FEDERATION_ENCRYPTION_KEY` parse matrix mutates a process-global env var; confined to a single sequential test in its own test binary so it can't race the others.
- **Risk — over-testing dependencies:** mitigated by targeting the composition the auth crate owns, not the store rows underneath it.

## References

- [[Clusters/Product Ladder 102+]] Phase XXI
- [[Clusters/Cluster 22.0]] (capabilities hardening)
- [[Retros/Cluster 111.0]], [[Architecture]]
