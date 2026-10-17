# Provider matrix

What you can plug in **without forking Maidan**. Two database *dialects*
(Postgres, SQLite), then many *hosts* that speak those dialects. Other
surfaces (embeddings, object store, IdP, mail) are already traits with
one or two implementations.

This page is the operator-facing matrix. The workstream that keeps it
honest is [Pre-Public Hardening.md](archive/Pre-Public%20Hardening.md) section
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
unsupported until someone measures it. LibSQL/Turso was measured and is not a
driver flag (below).

### Recipe: managed Postgres (Neon, RDS, Supabase)

Set `DATABASE_URL` to the host's connection string with TLS on, and start
Maidan. The migration runner creates the `vector`, `pgcrypto` and `uuid-ossp`
extensions itself (`CREATE EXTENSION IF NOT EXISTS`), so the role in the URL
must be allowed to create extensions. On all three, the owner role the
provider gives you is.

Two rules hold on every host:

- **Connect directly or through a session-mode pooler, never a
  transaction-mode one.** The event bus holds a `LISTEN` on one connection for
  as long as the server runs. A transaction-mode pooler (PgBouncer in
  transaction mode, Neon's `-pooler` endpoint, Supabase's port 6543) hands that
  connection to someone else between transactions, and the bus stops hearing
  `NOTIFY`.
- **Turn TLS on in the URL:** `?sslmode=require`, or `sslmode=verify-full` to
  check the certificate. The image carries its own roots
  ([Production](Production.md)).

| Host | `DATABASE_URL` | Status |
|------|----------------|--------|
| **Neon** | The **direct** endpoint, the host without `-pooler`: `postgres://<role>:<password>@ep-<id>.<region>.aws.neon.tech/<db>?sslmode=require` | Runs in production: `dev.maidan.world` is on a Neon Postgres 17 project, direct connection ([Production](Production.md), "Hosted on Cloudflare"). A Neon compute that scales to zero adds a cold start to the first query after it sleeps. |
| **Amazon RDS / Aurora PostgreSQL** | `postgres://<master user>:<password>@<instance>.<id>.<region>.rds.amazonaws.com:5432/<db>?sslmode=require` | Written, not run against an RDS instance. Check that the engine version offers pgvector (`SELECT name FROM pg_available_extensions WHERE name = 'vector'`). The master user holds `rds_superuser`, which can create the three extensions. Connect to the instance or cluster endpoint, not RDS Proxy, which shares connections between clients. |
| **Supabase** | The direct connection, `postgres://postgres:<password>@db.<ref>.supabase.co:5432/postgres?sslmode=require`, or the session pooler on port **5432** of the pooler host | Written, not run against a Supabase project. The direct host answers over IPv6 only unless the project has the IPv4 add-on; from an IPv4-only network use the session pooler. Never port 6543 (transaction mode). |

### LibSQL / Turso (measured: not a driver flag)

Measured on 2026-10-16 against `main` @ 466c2ec6. `DATABASE_URL` picks the
dialect by prefix, and only `postgres://`, `postgresql://` and `sqlite:` are
accepted:

- `libsql://<db>.turso.io?authToken=...` stops boot: "cannot detect dialect from
  connection URL".
- `file:/path/to.db` stops boot the same way.
- `sqlite:///path/to.db?mode=rwc` boots and serves `/health/ready`.

A remote Turso database speaks libSQL's own protocol over HTTP or WebSocket, and
the SQLite driver underneath (`sqlx`'s, linking libsqlite3) has no client for it.
Supporting Turso would mean a new driver, not a flag, so it stays out (this page's
rule: no third `Store`). A local libSQL file in plain SQLite format can be opened
as `sqlite:`, but this was not tested, and nothing syncs it to Turso.

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

### Recipe: local embeddings with Ollama

[`examples/embeddings/compose.ollama.yaml`](../examples/embeddings/compose.ollama.yaml)
is an overlay on the root `compose.yaml`. It adds Ollama serving
`nomic-embed-text` (768 dimensions) and points the server's
`openai-compatible` provider at Ollama's `/v1/embeddings`:

```sh
docker compose -f compose.yaml -f examples/embeddings/compose.ollama.yaml \
  --profile full up -d --build
```

A one-shot `ollama-pull` service downloads the model into a volume before
`maidan-server` starts, because the server probes the endpoint for the vector
dimension at boot. Search with `GET /workspaces/{wid}/search?q=...&mode=semantic`.
Each hit carries `embedding_model: "nomic-embed-text"`. Live indexing runs on
Postgres; on SQLite, semantic search needs the `sqlite-vec` feature
([Embeddings](Embeddings.md)).

Outside compose, the same three variables point any server at an Ollama on the
host: `MAIDAN_EMBEDDING_PROVIDER=openai-compatible`,
`MAIDAN_EMBEDDING_ENDPOINT=http://127.0.0.1:11434/v1/embeddings` and
`MAIDAN_EMBEDDING_MODEL=nomic-embed-text`.

**Needs a server with the fix "A remote embedding provider no longer blocks or
crashes the server"** (CHANGELOG). Before it, a debug build crashed at boot with
this provider, and a release build blocked a runtime worker for each embedding
call.

**Hugging Face text-embeddings-inference** serves the same OpenAI-style route,
so the overlay's three variables work with its `/v1/embeddings` in place of
Ollama's. Not run for this recipe.

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

The S3 backend sends path-style requests (`<endpoint>/<bucket>/<key>`) to the
endpoint you give it. It needs a static access key: `S3_ACCESS_KEY_ID` and
`S3_SECRET_ACCESS_KEY` are required, so an instance role or workload identity
alone is not enough. Both keys also take a `_FILE` form.

### Recipe: Cloudflare R2

Runs in production: `dev.maidan.world` keeps its artifacts in an R2 bucket
([Production](Production.md), "Hosted on Cloudflare").

1. Create the bucket, then an R2 API token with **Object Read & Write** on that
   bucket only. R2 shows the token's access key id and secret once.
2. Set:

```sh
ARTIFACT_BACKEND=s3
S3_ENDPOINT=https://<account id>.r2.cloudflarestorage.com
S3_BUCKET=<bucket>
S3_REGION=auto
S3_ACCESS_KEY_ID=<R2 access key id>
S3_SECRET_ACCESS_KEY=<R2 secret>
```

R2's region is `auto`. Leave the bucket private: the server
serves artifacts itself, behind workspace access.

### Recipe: AWS S3

Written, not run against an AWS account.

1. Create a private bucket (Block Public Access on), and an IAM user or role
   whose access key the server will hold.
2. Give it only what the backend calls: `HeadBucket` at boot; get, put, head and
   delete on objects; and the multipart calls (create, upload part, complete,
   abort).

```json
{
  "Version": "2012-10-17",
  "Statement": [
    { "Effect": "Allow", "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::<bucket>" },
    { "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload"],
      "Resource": "arn:aws:s3:::<bucket>/*" }
  ]
}
```

3. Set:

```sh
ARTIFACT_BACKEND=s3
S3_ENDPOINT=https://s3.<region>.amazonaws.com
S3_BUCKET=<bucket>
S3_REGION=<region>                 # the bucket's region, e.g. eu-west-1
S3_ACCESS_KEY_ID=<access key id>
S3_SECRET_ACCESS_KEY=<secret>
```

The endpoint must be the bucket's own region. AWS still serves path-style
requests. A bucket name with dots needs path-style anyway, because its name
breaks the virtual-hosted TLS certificate.

---

## Auth (humans)

Capability-scoped **bearer tokens** for agents. **OIDC** for humans
(`MAIDAN_OIDC_ENABLED=1` + issuer discovery). Any identity provider that
speaks standard OIDC should work. A self-hosted Keycloak
recipe, run end to end, is in [OIDC](OIDC.md) ("Recipe: Keycloak"): a
realm file, SCIM pre-provisioning with sign-in linked by verified email,
and a smoke script.

People can be provisioned over SCIM 2.0 (`/scim/v2/`). No SAML. Document
"use an IdP that speaks OIDC" rather than building enterprise SSO in-tree.

### Recipe: Google as the identity provider

Written from Google's OIDC discovery and Maidan's OIDC settings
([OIDC](OIDC.md), "Configuration"). Not run against a Google Cloud project.
Steps 3 and 6 of the Keycloak recipe, a SCIM member for each person and a token
for their own MCP client, apply unchanged.

1. In Google Cloud's **Google Auth Platform**, configure the consent screen. While
   it is in *Testing*, only the test users you list can sign in.
2. Create an OAuth client of type **Web application**. Its authorized redirect
   URI is your instance's callback, `https://<maidan host>/auth/oidc/callback`.
   Google accepts plain `http://` only for `localhost`. Google issues a client
   secret for this type and requires it on the code exchange, beside S256 PKCE.
3. Point Maidan at Google:

```sh
export MAIDAN_SESSION_SECRET=...                       # 32 random bytes or more
export MAIDAN_OIDC_ENABLED=1
export MAIDAN_OIDC_ISSUER=https://accounts.google.com
export MAIDAN_OIDC_CLIENT_ID=<id>.apps.googleusercontent.com
export MAIDAN_OIDC_CLIENT_SECRET_FILE=/run/secrets/google-oidc-secret
export MAIDAN_OIDC_REDIRECT_URI=https://<maidan host>/auth/oidc/callback
export MAIDAN_OIDC_LINK_EMAIL=1
```

Google's ID token carries `email` and `email_verified`, so a SCIM-provisioned
member whose handle is the person's Google address is linked on first sign-in.
Keep `MAIDAN_OIDC_AUTO_PROVISION` off: with it on, any Google account that can
get past the consent screen gets a member. Google publishes no
`end_session_endpoint`, so signing out ends the Maidan session only, and
`MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI` has nothing to do.

---

## Mail

SMTP only (`MAIDAN_SMTP_HOST`, `MAIDAN_SMTP_FROM`, …). Amazon SES,
SendGrid, Mailgun and Postfix all work **as SMTP relays**. Delivery is
durable: mail goes through the `maidan_mail_outbox` queue, which a background
worker drains and retries. There is no native SES or SendGrid HTTP driver.

---

## Event bus

Tied to the database dialect: in-memory (SQLite / tests) vs Postgres
`LISTEN/NOTIFY`. There is no Redis / NATS bus. Multi-process Maidan
implies Postgres.

---

## Still to prove (Hardening I, not new code)

- **I1 (done)** — this page; keep it true when env vars change. (written 2026-08-25)
- **I2 (written 2026-10-16)** — embedding: the Ollama compose overlay
  ("Recipe: local embeddings with Ollama"). TEI is described but not run.
- **I3 (written 2026-10-16)** — R2 runs in production. AWS S3 is written, not
  run.
- **I4** — Keycloak done (2026-10-08, [OIDC](OIDC.md)). The SaaS recipe (Google)
  is written, not run.
- **I5 (written 2026-10-16)** — Neon runs in production. RDS and Supabase are
  written, not run.
- **I6 (measured 2026-10-16)** — LibSQL/Turso is not a driver flag, so it is not
  implemented.

---

## See also

- [Handoff.md](archive/Handoff.md) — session pickup
- [Protocols.md](Protocols.md) — wires (MCP/A2A/REST), not hosts
- [Embeddings.md](Embeddings.md)
- [Production.md](Production.md)
- [Deploy.md](Deploy.md)
- [Pi.md](Pi.md)
- [Pre-Public Hardening.md](archive/Pre-Public%20Hardening.md) (section I)
- [Path to Impressive.md](archive/Path%20to%20Impressive.md) (ecosystem / provider matrix)
