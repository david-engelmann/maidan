# Cluster 189.0 retro — the encryption key can rotate now

> Tag **`v189.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc B (multi-tenant SaaS operability), **finale**.

## What shipped

- A try-all-keys decrypt keyring in `maidan-auth`: `decrypt_peer_secret_rotating`
  tries the primary then `FEDERATION_DECRYPT_KEYS` fallbacks; the 6 decrypt call
  sites use it; `main.rs` loads the fallbacks at startup.

## Surprises / decisions

- **AEAD turns "try every key" from scary into correct.** The instinct is that
  guessing keys is dangerous, but an authenticated cipher rejects the wrong key
  with a tag mismatch — it *cannot* return plausible-but-wrong plaintext. That
  property is what makes a no-format-change keyring safe, and it's the whole
  reason this cluster is small and low-risk instead of a delicate versioned-blob
  migration.
- **No ciphertext-format change was the win.** A key-id-prefixed blob format
  would have needed a migration for every existing (unprefixed) ciphertext and a
  legacy-decode path — more code and more ways to strand a secret. Trying keys in
  order needs none of that: an old blob decrypts with whichever key made it.
- **The dangerous failure mode is a *silently dropped* old key.** So parsing
  `FEDERATION_DECRYPT_KEYS` is strict — a malformed entry is a hard startup error,
  not a skipped key. Skipping would quietly make that key's secrets
  undecryptable, discovered only when someone tries to use one.

## Decisions

- **Encrypt stays single-key; only decrypt is a keyring.** New writes always use
  the current primary, so the fleet converges on the new key naturally; the
  keyring only exists to read what the old key wrote. This kept the change to the
  decrypt sites — encrypt sites and the four runtime `encryption_key` fields are
  untouched.
- **Lazy migration, documented.** No bulk re-encrypt job — a secret moves to the
  new key when it's next saved. The old key stays in `FEDERATION_DECRYPT_KEYS`
  until then. A re-encrypt sweep is a tracked follow-up, not a blocker.

## Capability table extension

| Change | Where |
|--------|-------|
| Secret-rotation keyring (try-all decrypt + `FEDERATION_DECRYPT_KEYS`) | `maidan-auth/src/peer_secret.rs` |

## Risks identified + still open

- **Net risk-reducing, backward-compatible.** No fallbacks set → behaviour is
  byte-identical to before. Open (Open Work): lazy migration (no bulk re-encrypt
  sweep); the fallback set is a startup `OnceLock` (rotation needs a restart, not
  a live reload).

## Forward look

**Arc B (multi-tenant SaaS ops) is complete** (185 Helm, 186 retention, 187
export, 188 usage, 189 rotation). Next: **Arc C — agentic task-queue depth**
(assignment read-side list-mine/claim-next, claim leases, `roots/list`,
structured tool-call transcripts, `wait_for_mention`, handoff notes, federation
`parts→content`).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
