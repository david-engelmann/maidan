### A released claim takes its holder's status with it, and release_own_claims gets a REST twin

- **Added:** `POST /workspaces/{id}/claims/release-own` (optional body `{"note": "..."}`, `thread:transition`) is the REST twin of MCP `release_own_claims`: it releases every live leased claim the calling member holds in its own workspace and answers `{"released": [thread_id, ...]}`. The member is the token's; another workspace is refused with `403`.
- **Changed:** A claim that ends by `release_claim`, `release_own_claims`, the reaper or a `claim_next` takeover of a lapsed lease drops the status its holder declared, in the same transaction, so the board no longer shows a departed holder's "working". A status the thread's owner declared stays.
- **Changed:** The MCP server's `instructions` tell a claimant to call `release_own_claims` once per workspace at startup, before its first claim, unless another live process shares its member token.
- **Added:** The TypeScript, Python and Go SDKs (0.3.0, untagged) call it: `releaseOwnClaims(workspaceId, note?)`, `release_own_claims(workspace_id, note=None)` and `ReleaseOwnClaims(workspaceID, note)`. The Rust SDK does not have it yet.
