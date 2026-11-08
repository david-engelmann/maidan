### The Rust SDK releases a restarted waiter's claims

- **Added:** The Rust SDK (0.3.0, untagged) calls `POST /workspaces/{id}/claims/release-own`: `Client::release_own_claims(workspace_id, note: Option<&str>)` returns `ReleasedClaims { released }`, the thread ids given back to the queue. All four SDKs now have it.
