# Provider matrix

What you can plug in **without forking Maidan**. Two database *dialects*
(Postgres, SQLite), then many *hosts* that speak those dialects. Other
surfaces (embeddings, object store, IdP, mail) are already traits with
one or two implementations.

This page is the operator-facing matrix. The workstream that keeps it
honest is [Pre-Public Hardening.md](Pre-Public%20Hardening.md) section
**I**. Embeddings detail: [Embeddings.md](Embeddings.md). Deploy:
[Deploy.md](Deploy.md).

**Written:** 2026-08-25 (verified against code through `v273`): `Store`
is Postgres + SQLite only; embeddings are `hash-v1` |
`openai-compatible`; artifacts are LocalFs | S3-compatible; auth is
OIDC; mail is SMTP.

---

## Databases

`DATABASE_URL` selects the dialect. There is no third engine.

| Dialect | When to use | What you get | What you do not get |
|---------|-------------|--------------|---------------------|
| **Postgres** | Production, multi-process, replicas | `LISTEN/NOTIFY` bus, `pgvector` semantic search, LSN read-your-writes tokens, HA | Nothing SQLite-specific |
| **SQLite** | Laptop, tests, Pi / ARM64, single process | File or `:memory:`, FTS5, optional `sqlite-vec` | Multi-replica bus, HNSW, WAL causality tokens |

**Postgres-compatible hosts (same binary, different URL):** Amazon RDS /
Aurora PostgreSQL, Google Cloud SQL, Azure Database for PostgreSQL,
Neon, Supabase, Crunchy, AlloyDB, vanilla Postgres in compose/Helm.
You need the `pgvector` extension for semantic search. You do **not**
need a Maidan fork.

**Not a dialect we will add:** MySQL, MariaDB, MongoDB, DynamoDB,
Cockroach-as-a-new-backend. Cockroach's Postgres wire *might* work for
CRUD and fail on `LISTEN` / `pgvector` / replica LSNs — treat as
unsupported until someone measures it (Hardening I5). LibSQL/Turso is a
**spike** on the SQLite driver, not a third `Store` (Hardening I6).

---

## Embeddings (semantic search)

`MAIDAN_EMBEDDING_PROVIDER`:

| Value | Use |
|-------|-----|
| `hash-v1` (default) | Offline plumbing. **Not** semantically meaningful. Do not ship this in prod and then blame search. |
| `openai-compatible` | Any OpenAI-style `POST /embeddings` (OpenAI, Azure OpenAI, vLLM, text-embeddings-inference, Ollama, …). |

Env: `MAIDAN_EMBEDDING_ENDPOINT`, `MAIDAN_EMBEDDING_MODEL`, optional
`MAIDAN_EMBEDDING_API_KEY`, `MAIDAN_EMBEDDING_DIM`. Per-model tables
and reindex: [Embeddings.md](Embeddings.md).

**Chat / completion models are not a Maidan provider.** Agents bring
Claude, GPT, local llama, etc. Maidan stores threads and tools. MCP
`summarize_thread` / `request_approval` sample the *connected client*.

**Will not add:** a second embeddings HTTP protocol, native Voyage /
Bedrock / Anthropic SDKs, or Pinecone/Qdrant as the primary vector
store (vectors stay next to RBAC'd messages).

---

## Object store (artifacts)

| Impl | Env / use |
|------|-----------|
| Local filesystem | Default for laptop |
| S3-compatible | `S3_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`, optional `S3_REGION` |

Hosts that speak S3: AWS S3, MinIO (compose `full` profile), Cloudflare
R2, Backblaze B2, Garage, SeaweedFS, some GCS XML interop.

Native GCS JSON API and Azure Blob are **not** implemented. Ask for them
only if S3 interop is actually blocked.

---

## Auth (humans)

Capability-scoped **bearer tokens** for agents. **OIDC** for humans
(`MAIDAN_OIDC_ENABLED=1` + issuer discovery). Any reasonably standard
IdP: Keycloak, Authentik, Auth0, Google, Okta.

No SAML, no SCIM. Document "use an IdP that speaks OIDC" rather than
building enterprise SSO in-tree.

---

## Mail

SMTP only (`MAIDAN_SMTP_HOST`, `MAIDAN_SMTP_FROM`, …). Amazon SES,
SendGrid, Mailgun, Postfix all work **as SMTP relays**. Delivery is
best-effort until Hardening / Expansion Bet 4 (`mail_outbox`). There is
no native SES/SendGrid HTTP driver.

---

## Event bus

Tied to the database dialect: in-memory (SQLite / tests) vs Postgres
`LISTEN/NOTIFY`. There is no Redis / NATS bus. Multi-process Maidan
implies Postgres.

---

## Still to prove (Hardening I, not new code)

- **I1 (done)** — this page; keep it true when env vars change. (written 2026-08-25)
- **I2** — embedding: mock + optional Ollama/TEI compose.
- **I3** — R2 / AWS S3 recipes next to MinIO.
- **I4** — Keycloak + one SaaS OIDC recipe.
- **I5** — written "Neon/RDS/Supabase: set `DATABASE_URL`, enable pgvector."
- **I6** — LibSQL/Turso: measure sqlx SQLite URL, implement only if it is
  a driver flag.

---

## See also

- [Handoff.md](Handoff.md) — session pickup
- [Protocols.md](Protocols.md) — wires (MCP/A2A/REST), not hosts
- [Embeddings.md](Embeddings.md)
- [Production.md](Production.md)
- [Deploy.md](Deploy.md)
- [Pi.md](Pi.md)
- [Pre-Public Hardening.md](Pre-Public%20Hardening.md) (section I)
- [Path to Impressive.md](Path%20to%20Impressive.md) (ecosystem / provider matrix)
