# Cluster 111.0 retro — `maidan-auth` test suite

> Tag **`v111.0.0`**. First cluster of Phase XXI (correctness & coverage); **opens the phase**.

## What shipped

- **`maidan-auth` integration suite** — the authn/authz crate had only five
  inline unit tests and no `tests/` directory despite owning capability
  checks, constant-time bearer comparison, and at-rest peer-secret
  encryption. Added **26 integration tests** across the four exit-criteria
  areas. (111.0.1, #308)
  - **`capability_matrix.rs`** (14, pure) — capability vocabulary
    (`is_known`, `validate_list` flags the first unknown, `validate_subset`
    rejects unknown-before-grant and blocks escalation beyond the
    installation grant, `default_minted` is a known non-admin subset) +
    the `AuthContext` authorization matrix across `from_token` /
    `from_app_token` / `from_session` / `bypass` (capability grant/deny,
    `require_capability` → `Forbidden`, app-installation carry,
    cross-workspace `ensure_workspace` scoping, bypass short-circuiting) +
    constant-time `hashes_equal` (identical-only match, length-mismatch
    guard must not panic / index past the stored digest).
  - **`peer_secret_aead.rs`** (7) — ChaCha20-Poly1305 round-trip, nonce
    randomizes ciphertext, **tamper detection** on both the ciphertext body
    and the prepended nonce (Poly1305 rejects each → `DecryptFailed`),
    truncation and non-base64 → `InvalidCiphertext`, wrong key →
    `DecryptFailed`, and the `FEDERATION_ENCRYPTION_KEY` parse matrix
    (hex / base64 / wrong-length / garbage / missing).
  - **`token_lifecycle.rs`** (5, store-backed) — `resolve_bearer` against
    an in-memory SQLite store: capability propagation into the context,
    forged-secret rejection, **post-revocation** and **post-expiry**
    failure, plus `resolve_peer_bearer` round-trip + forged rejection.
- **Dev-deps** — `sqlx` + `chrono` (store-backed lifecycle test) and
  `rt-multi-thread` on the `tokio` dev-dep. No `src/` or
  production-dependency changes — tests only.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| (covered elsewhere) | End-to-end `resolve_bearer` **app-token branch** (real installation FK) | The `from_app_token` context construction is unit-tested directly in `capability_matrix.rs`; full app-token resolution through HTTP is covered by `agent_apps_e2e` / `app_oauth_e2e` in the server crate. Re-staging an installation FK inside the auth crate added setup without new signal. |
| Cluster 112  | FSM property tests (`maidan-fsm` proptest) | Next cluster of Phase XXI. |
| Cluster 114  | Coverage-floor ratchet (`COVERAGE_MIN_LINES` 11→25→40) | This suite raises auth coverage but the floor is bumped as a dedicated step once 112–113 land. |

## Surprises

- **`hashes_equal("", "")` is `true`.** The length guard only short-circuits
  *unequal* lengths; two equal-length (here zero-length) inputs fall through
  to `subtle`'s constant-time compare, which reports empty-equals-empty as
  equal. Pinned with an explicit assertion so the contract is documented
  rather than incidental — a stored hash is always 64 hex chars in practice,
  so this edge can't be reached by a real bearer, but the function's contract
  is now nailed down.
- **Store-level token CRUD was already covered** in
  `maidan-store/tests/api_tokens.rs` (create / lookup / revoke / expiry).
  Rather than duplicate it, this suite targets the *composition* the auth
  crate owns — `hash_secret` → active-by-hash lookup → constant-time compare
  → context mapping — which had zero coverage.

## Decisions

- **Test the seam each crate owns, don't re-prove its dependencies.** The
  auth suite exercises `resolve_bearer`'s end-to-end behavior (including
  revocation/expiry *propagation*) against a real store, but leans on
  `maidan-store`'s own tests for the row-level CRUD. No [[Architecture]]
  change.
- **SQLite `sqlite::memory:` for the store-backed tests** — always available,
  so no Docker/testcontainers skip guard is needed (unlike the Postgres
  integration suites). Backend-agnostic resolution logic means one backend
  is sufficient signal here.

## Capability table extension

| Capability | Where |
|------------|-------|
| Capability-vocabulary + `AuthContext` authorization matrix coverage | `maidan-auth/tests/capability_matrix.rs` |
| Peer-secret AEAD round-trip / tamper / key-parse coverage | `maidan-auth/tests/peer_secret_aead.rs` |
| Bearer lifecycle (mint / revoke / expire / forge) coverage | `maidan-auth/tests/token_lifecycle.rs` |

## Risks identified + mitigated

- **Tamper-resistance was previously unproven in-crate.** The AEAD round-trip
  had an inline test, but nothing asserted that a flipped ciphertext or nonce
  byte is *rejected*. Both are now regression-guarded.
- **Constant-time compare length edge.** A short candidate hash could in
  principle index past a stored digest; the guard is now explicitly tested.

## Risks identified + still open

- **App-token resolution** in the auth crate is only covered transitively
  via the server e2e suite (see "deferred").

## Forward look

Phase **XXI (correctness & coverage)** continues with **Cluster 112 — FSM
property tests** (`maidan-fsm` thread-lifecycle invariants under `proptest`),
then backend parity (113), a coverage-floor ratchet + fuzz (114), and a
module-split / `unwrap()` purge (115).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
