### A per-peer private egress allowlist

- **Added:** `MAIDAN_PRIVATE_EGRESS_HOSTS` names the peers (a federation peer, an A2A push receiver, a webhook) that may live on a private network. A listed host may resolve to a private address in production; every other target keeps the public-only rule, and a listed host that resolves to a link-local address (cloud metadata) is still refused. It replaces the whole-guard `MAIDAN_ALLOW_PRIVATE_EGRESS` for real peers, which stays development-only.
