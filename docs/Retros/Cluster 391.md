# Cluster 391 retro — Wave 3 #31: signed workspace export

Wave 3 #31 (B7) asked for a **signed workspace export** a blank GHCR
instance can verify without calling the origin host, plus a documented
token-continuity policy.

This is **not** Room-LSN (Cluster 390) and **not**
`Maidan-Consistency-Token` (Cluster 263). Those tokens answer projector
lag and read-your-writes. The export envelope answers "did this file
leave the origin intact?"

Three impl PRs (391.1–391.3) + this retro. Every PR targets `main`.
**Row #31 is closed.** Do not start Wave 3 #32–36 from this close.

## What shipped

- **391.1 (#844) — envelope + Ed25519.** `$type`
  `maidan.workspace.export/1`, `TokenPolicy::TokensDieOnExport` (the
  only policy), canonical JSON + SHA-256 statement hash, forbidden
  secret-field walk. Operator key is a 32-byte seed
  (`MAIDAN_EXPORT_SIGNING_KEY`, hex or base64). Verify uses the
  embedded public key; `MAIDAN_EXPORT_VERIFY_KEYS` is an optional
  authenticity pin.
- **391.2 (#845) — REST.** `GET /workspaces/:id/export` now returns the
  signed envelope (refuses if the key is unset — never unsigned).
  `POST /workspaces/export/verify`, `POST /workspaces/import` (signed
  body only), `GET /operator/export-public-key`. All `token:admin`.
  Import remaps (`mode=new`) or restores ids (`mode=restore`, `force`
  erases first).
- **391.3 (#846) — MCP.** `export_workspace` /
  `verify_workspace_export` / `import_workspace`. Assemble lives in
  `maidan-store::build_workspace_export`; flatten/remap live in
  `maidan-types` so REST and MCP sign the same graph.

## Decisions

- **Tokens die on export.** Continuity is unsafe: API token hashes,
  webhook/slash/OIDC secrets, and at-rest AEAD keys do not travel, and
  stuffing them back in is a verification failure. After import the
  operator mints new tokens on the destination (`token:admin`).
- **Operator key, not a per-workspace key.** One host-level Ed25519
  seed signs every workspace the operator is willing to carry. A
  stranger verifies integrity offline from the embedded public key. A
  destination that wants *authenticity* pins expected keys in
  `MAIDAN_EXPORT_VERIFY_KEYS`; empty pin = blank-instance default
  (tamper-evident, not a trust anchor).
- **Fail closed.** Missing signing key → export 400. Bit-flip → hash
  mismatch. Bad signature / wrong pin / secret fields → 400. No
  unsigned fallback, no origin HTTP callback.
- **`$type` is the contract.** Breaking changes are
  `maidan.workspace.export/2`, not a silent reshape of `/1`.
- **Do not conflate with Room-LSN or Consistency-Token.** Different
  value space, gating, and purpose.

## Surprises

- **Import now requires the signed envelope.** Cluster 270 accepted a
  bare `WorkspaceExport`. Greenfield: no compatibility shim.
- **`sign_export` takes only `(key, payload)`.** Token policy is
  stamped by the signer, not the caller, so it cannot be rewritten
  after the fact.
- **Assemble had to move out of the server crate** before the MCP
  twin, or the two surfaces would drift on the next field add.

## Test evidence

- Types: canonical JSON, statement omit-hash, forbidden fields, token
  policy wire is `tokens_die_on_export`.
- Auth: sign→verify, bit-flip, bad sig, stuffed `token_hash`, wrong
  pin.
- Server e2e: blank-instance verify+import (no origin callback, no
  dest signing key), bit-flip / bad sig, key pin, missing key, public
  key route. Imported member has **zero** API tokens.
- MCP: export → verify on a second blank store → import `mode=new`;
  bit-flip and stuffed secrets fail; missing key fails closed.
  Catalog + capability-map contracts.

## Forward look

**Cluster 391 is complete. Row #31 is closed.**

Do not start Wave 3 #32–36 from this retro. #32 is a hash-chained log
+ strong refs; #33 snapshot catch-up; #34 tombstone explorer; #35
named capability sets + `maidan://` URIs; #36 WASI slash-handler.

Deferred: streaming / paginated export for huge workspaces; MCP
`export_public_key` (REST already has `GET /operator/export-public-key`);
artifact-blob packing (187 still omits content-addressed bytes);
reactions/votes in the bundle (187 gap).

Do **not** cut `v391.0.0` from this PR — the maintainer tags, which
triggers `release.yml`.

## Acknowledgements

#844 types+Ed25519 → #845 REST → #846 MCP → this retro.
