# Cluster 371 retro — Wave 2 #19: secret-ref (G19 + T3)

Wave 2 #19 gives the room a **secret-ref**: a named secret whose *value never
enters the event log*. The log — a message, a webhook payload, a tool argument —
carries a `secret://<name>` **reference**; the store holds the (encrypted) value;
and the value is materialized only at the moment it's needed — Pi resolves it at
exec, or the egress broker substitutes it on the way out to an allowlisted host.

## What shipped

- **371.1 (#741) — the store foundation.** `maidan_secrets` (pg 0076 / sqlite
  0075) storing only `value_ciphertext`; `Secret` (metadata only) / `NewSecret` /
  `SecretId`; the pure `secret://` ref helpers (`secret_refs_in`,
  `substitute_secret_refs` — resolve known refs, **leave unknown ones literal +
  report them**, `is_valid_secret_name`); and the `SecretStore` CRUD (create
  upserts = rotation), both backends. Zero-blast-radius.
- **371.2 (#742) — REST.** `secret:read` / `secret:admin` capabilities; create
  (encrypts) / list (metadata) / **resolve** (decrypts → the value; "Pi fetches at
  exec") / delete. The value crosses the wire only on create + resolve.
- **371.3 — MCP.** `list_secrets` / `resolve_secret` (the agent-native resolve).
  `McpServer` gained an `encryption_key` OnceLock (set at startup, the
  slash_dispatcher pattern) so the tool can decrypt without touching `new()`.
- **371.4 — the egress SecretBroker.** On a webhook delivery, `secret://<name>`
  refs are substituted with the real value **only when the target host is on
  `MAIDAN_SECRET_EGRESS_ALLOWLIST`**; a non-allowlisted host gets the literal ref.

## Decisions

- **The log holds a reference, the store holds the value.** A `secret://name` in
  a message body is just text — it's never expanded at post/append time, so the
  event log never contains a secret. Expansion happens at the edge: the resolve
  endpoint (into a response) or the broker (into an outbound request), both
  transient.
- **The route layer owns the key, the store is key-agnostic.** `maidan_secrets`
  stores ciphertext; encryption/decryption uses the Cluster-189 AEAD keyring at
  the route/broker layer. The store never sees plaintext, and metadata reads
  never even SELECT the ciphertext column.
- **Two capabilities, split read from admin.** `secret:read` (resolve/list) is the
  "Pi fetches at exec" grant; `secret:admin` (create/rotate/delete) is a separate,
  higher bar — a resolver token can't mint or destroy secrets. Both are
  granted-on-purpose (not in `default_minted`).
- **The broker fails safe.** Substitution happens only for an allowlisted host,
  only when a key is configured, and only for a known secret; in every other case
  the ref is **left literal, never blanked** — a secret is never leaked to an
  untrusted endpoint, and a missing secret never silently becomes an empty string.
- **Substitute at send, not at enqueue.** The broker runs in `poll_deliveries`
  just before `deliver_http`, so the plaintext never persists in the delivery
  queue and the HMAC signature covers the substituted body; a retry re-substitutes
  from the unchanged queued payload.

## Surprises

- **`McpServer` has no key, and `new()` has ~40 call sites.** Threading the key
  through the constructor would be a wide ripple; a `OnceLock` + `set_encryption_key`
  (set once at startup, unset in tests) mirrors `slash_dispatcher` and keeps
  `new()` untouched.
- **The env allowlist is a cached global.** A `OnceLock` cache means a test can't
  vary it — so the async substitution has an allowlist-parameterized `substitute_with`
  core (public for testing) behind the env-reading `substitute_for_egress`.
- **The `context_query_count_e2e` flake took three reruns** on 371.1's integration
  job (the count flip-flops ±1 on a connection warm-up). Documented, unrelated to
  secrets — but it's now recurring often enough on the *required* integration job
  to be worth a proper pool-warm-up fix (a follow-up).

## Test evidence

- maidan-types: 6 pure ref-helper unit tests.
- Store: `secrets` (both backends) — CRUD + rotate-in-place.
- Server: `secret_rest_e2e` (encrypt→decrypt round-trip, no-value-in-list,
  read-can't-create, 404/400), `secret_broker_e2e` (allowlisted → substituted,
  non-allowlisted → byte-for-byte unchanged), the broker's 3 pure unit tests, the
  bijection / matrix / openapi↔map contracts.
- MCP: `secret_tools_list_and_resolve` + the catalog / capability-map contracts.

## Forward look

**Wave 2 #19 is complete.** Deferred (follow-ups): per-workspace egress
allowlists (env-global today); the broker on other egress paths (A2A, automation
deliveries) — webhooks demonstrate the pattern; secret-ref resolution inside
slash-command / automation templates; a secret audit trail (who resolved what).
**Next: Wave 2 #20** (G17 + B25 — a freeze-member kill-switch: drop leases, refuse
`claim_next`, leave a gate; + a catalog of `MAIDAN_*` kill-switch flags).

## Acknowledgements

Four impl PRs (#741 store → #742 REST → 371.3 MCP → 371.4 broker) plus this retro,
on the foundation-then-wire + new-route-preflight + capability-registry patterns.
