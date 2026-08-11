# Cluster 189.0 — SaaS ops: secret-rotation keyring

**Theme:** Arc B (multi-tenant SaaS operability), finale — let an operator rotate
the at-rest encryption key without stranding existing secrets.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v189.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Try-all-keys decrypt keyring + `FEDERATION_DECRYPT_KEYS` fallbacks | `maidan-auth/src/peer_secret.rs` |
| 6 decrypt call sites → `decrypt_peer_secret_rotating` | `maidan-server/src/{slash_commands,fsm_hooks,webhooks,federation}.rs` |
| Startup init of the fallback keys | `maidan-server/src/main.rs` |

## Why

Secrets at rest (federation peer bearer tokens, webhook / slash / fsm-hook
secrets) are AEAD-encrypted with a single key from `FEDERATION_ENCRYPTION_KEY`.
There was **no rotation path**: changing that env var immediately makes every
stored ciphertext undecryptable. A workspace running for major companies must be
able to rotate a leaked/aged key.

## The fix

A **try-all-keys keyring** — the safest rotation design because it needs *no
ciphertext-format change* (fully backward-compatible):

- `encrypt` still uses the single primary key (unchanged).
- `decrypt_peer_secret_rotating(blob, primary)` tries the primary first, then
  each key in a process-wide fallback set (`FEDERATION_DECRYPT_KEYS`, parsed at
  startup). AEAD authentication makes trying the wrong key **safe** — it fails
  cleanly (tag mismatch) rather than returning garbage — so a keyring can attempt
  several keys without any risk of corruption.

Rotation flow: set the new key as `FEDERATION_ENCRYPTION_KEY` (new encrypts + the
first decrypt attempt) and move the old key into `FEDERATION_DECRYPT_KEYS`.
Existing ciphertexts decrypt via the old fallback; new ones via the new primary;
as secrets are re-saved they migrate to the new key, and the old key can be
dropped once none remain.

## Exit criteria

- A ciphertext made with a pre-rotation key still decrypts after the primary is
  rotated (via a fallback); no ciphertext-format change; a malformed fallback key
  fails startup rather than silently stranding secrets — **met**.
- `v189.0.0` tagged.

## Verification & limits

- `maidan-auth`: unit tests (`multi_tries_keys_until_one_works`,
  `parse_decrypt_keys_reads_a_list_and_rejects_bad`) + an end-to-end
  `keyring_rotation` integration test (old-key ciphertext decrypts under a
  rotated primary; post-rotation ciphertext decrypts on the first try; an unknown
  key fails). Federation / webhook / slash / fsm decrypt suites stay green (the
  rotating swap is transparent when no fallbacks are set).
- Limits (tracked): migration to the new key is **lazy** (a secret moves only
  when re-saved) — there's no bulk re-encrypt job yet, so an old key must stay in
  `FEDERATION_DECRYPT_KEYS` until every secret has been rotated. The fallback set
  is process-wide (a `OnceLock` set at startup); changing keys needs a restart.

## References

- [[Retros/Cluster 189.0]]; `maidan-auth/src/peer_secret.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Arc B).
