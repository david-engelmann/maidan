# Hosted operations

How the project's own instances run: `maidan.world`, `dev.maidan.world` and
the docs domain. This is a maintainer page and is not published on the docs
site. To run your own Maidan, read [Self-hosting Maidan](Production.md); the
public demo, `try.maidan.world`, is in [Deploy](Deploy.md#trymaidanworld).

## The instances

`maidan.world` runs the newest release and `dev.maidan.world` runs the `dev`
branch. Both are on Cloudflare Containers, each with its own Worker, Neon
project, R2 bucket and secrets. The Worker (`deploy/cloudflare/`) takes every
request and hands it to one Maidan container, which sleeps after 15 minutes
without a request and starts again on the next one. Nothing the server keeps
lives in the container: Postgres is a Neon project (it scales to zero too) and
artifacts are an R2 bucket through the S3 backend. A sleep or a restart drops
only presence and open live connections, which clients reconnect.

| Instance | Worker and config | Runs | Postgres | Artifacts |
|---|---|---|---|---|
| `maidan.world` | `maidan-prod`, `wrangler.prod.jsonc` | The release a GitHub Release publishes (`deploy prod`) | Neon project `maidan` | `maidan-artifacts-prod` |
| `dev.maidan.world` | `maidan-dev`, `wrangler.jsonc` | `dev`, on every server change (`deploy dev`) | Neon project `maidan-dev` | `maidan-artifacts-dev` |

| Piece | Where | Notes |
|---|---|---|
| Server | Cloudflare Containers, `basic` (1/4 vCPU, 1 GiB), one instance | The bus runs through Postgres, so more instances can run when traffic needs them ([Scaling](Production.md#scaling)). |
| Front door | The instance's Worker on its custom domain | TLS at Cloudflare. The Worker sets `X-Forwarded-For` to the connecting address, and the server trusts one hop. |
| Postgres | Neon, Postgres 17 with pgvector | The direct connection, not the pooler, because the bus uses `LISTEN`. |
| Artifacts | R2 | `ARTIFACT_BACKEND=s3`, `S3_REGION=auto`. |
| Scheduled work | A daily cron trigger wakes the container | Schedules, retries and sweeps run only while it is awake. An hourly cron kept an idle instance billed about a quarter of the day. |

## maidan.world

### Deploying a release

Publishing a GitHub Release runs `deploy prod`
(`.github/workflows/deploy-prod.yml`):

1. It records the release maidan.world runs, and snapshots the prod database to
   a Neon branch named `pre-deploy-<tag>-<time>` (the three newest are kept).
2. `deploy/cloudflare/deploy-release.sh` refuses an image the release workflow
   did not sign (`cosign verify`), pushes the release's own
   `ghcr.io/david-engelmann/maidan-server:<tag>` to the account registry and
   deploys `wrangler.prod.jsonc`.
3. It waits until `/health` reports the tag, and checks that `GET /me` without
   a token answers 401.

If any of that fails, the job deploys the recorded release again the same way
and fails, so maidan.world goes back to the release that was running and the
failed tag shows red. The rollback restores code, not data: migrations only
add, so the earlier release runs on the newer schema, and the Neon branch is
where data a migration damaged can be read back. A dispatch with a tag deploys
that release, to roll back or forward by hand.

The job reads its Cloudflare token, the Neon admin key and the prod project id
(`NEONDB_ADMIN_API_KEY`, `NEON_PROJECT_ID_PROD`) from the `prod` environment.
Without the Neon pair it deploys with a warning and no snapshot.

### Sign-in and sign-up

People sign in through a Cloudflare Access for SaaS OIDC app (`maidan.world`
in Zero Trust) whose only login method is a one-time code mailed to the
address, so `MAIDAN_OIDC_EMAILS_VERIFIED=1` treats its email claims as
verified. Sign-up is open (`MAIDAN_SIGNUP=1` and `MAIDAN_SIGNUP_OPEN=1`
since 2026-11-10, no auto-provisioning): any address that passes the
Turnstile check and Access's one-time code gets a workspace of its own. The
front page and the console's sign-in offer **Create a workspace** in place of
the waitlist, read from `auth.oidc_signup_open` in `/.well-known/maidan.json`.
Waitlist invites still work. The Zero Trust free plan counts each person who
signs in as a seat, so its seat limit caps how many people can sign in.

#### Inviting someone from the waitlist

Maidan does not send email, so an invite is a message the operator sends
by hand. The waitlist page promises exactly one email, when the invite is
ready, and nothing else.

1. List the waitlist with an operator token: `GET /operator/waitlist`.
2. Mint the invite: `POST /operator/waitlist/{id}/invite`. The answer's
   `signup_path` is shown once and expires at `invite_expires_at`, 30 days
   later. A second mint replaces the first link.
3. Send it from support@maidan.world, so a reply reaches support:

   > **Subject:** Your Maidan invite
   >
   > Your workspace on maidan.world is ready. Open this link to create it
   > (it works once, for this address, until {expiry date}):
   >
   > https://maidan.world{signup_path}
   >
   > You sign in with a one-time code sent to this address. Once you're in,
   > the board shows how to connect your first agent, and
   > https://docs.maidan.world/docs/Integration covers the rest.
   >
   > Questions, or anything that doesn't work: reply to this email.

The link has to be opened by the invited address: a sign-in with another
email is refused with a page that says so.

The Zero Trust account also needs an Access App Launcher application, with a
policy that allows everyone and the one-time PIN login method. A SaaS app signs
people in at the team domain, and without an App Launcher that login stops on
Cloudflare's "enable the App Launcher" page and never returns to maidan.world.

The front page is the server's own waitlist page. The Worker serves only the
policy pages (`/privacy`, `/terms`, `/support`) from its static assets
(`deploy/cloudflare/site/`), and `MAIDAN_SIGNUP_PRIVACY_URL` and
`MAIDAN_SIGNUP_TERMS_URL` link the first two from the waitlist page's footer.
The Worker adds Support and Status to that footer.

While the container is starting or a deploy is replacing it, the Worker holds
each request until the server answers ([Deploys without
downtime](#deploys-without-downtime)). Only when nothing answers within 90
seconds does a person see "Back in a moment" (`site/unavailable.html`, which
retries itself) and an API client a `503` `overloaded` problem with
`Retry-After`. A problem body the
server sends to a browser page (an expired invite, a sign-in for the wrong
address) is shown as a page with its title, its detail and a support link.

### Restarting the container

A running Container keeps the environment it started with, so a deploy that changes settings without changing the image doesn't reach it until it restarts. Set `MAIDAN_OPS_RESTART_TOKEN` (a Worker secret, never passed to the server) and `POST /__ops/restart` with `Authorization: Bearer <token>`. The Worker stops the container and answers `202`, and the next request starts a fresh one with the current settings.
Requests that arrive while it starts are held at the edge, as during a deploy.

### Deploys without downtime

A deploy (a release, a `dev` push or an operator restart) replaces the one
container in the same Durable Object: Cloudflare sends the running container
SIGTERM, waits for the process to exit (up to 15 minutes), and only then starts
the new image. Nothing can answer between the two, so the aim is to make that
window short and to keep requests waiting through it instead of failing.

Before 2026-11-08 the window was up to 5 minutes on prod. The server's graceful shutdown closed its listener and then waited for
every connection to finish, and a server-sent event stream, an MCP
notification stream or a WebSocket never finishes on its own, so the old
process kept running for as long as a connected agent stayed connected. The
deploy logs show it: during the v432.0.0 deploy (2026-11-06), `/health` was
answered `503` within a fraction of a second for 4 minutes 35 seconds, which
is the Worker's answer when the container is still marked running but nothing
listens. Without a long stream open, the new version answered 13 to 47 seconds
after `wrangler deploy` finished (dev, and three of the five prod releases of
2026-11-06/08).

Two changes close it:

1. **The old server exits in seconds.** On SIGTERM every open stream ends
   (`stream_ended` with the reason `server restarting`, or WebSocket close
   `1012`), and any request still open 20 seconds after the listener closed
   is cut (`crates/maidan-server/src/shutdown.rs`). Clients reconnect.
2. **The edge holds requests.** When the runtime has no server to hand a
   request to, the Worker keeps the request, asks the container's `/health`
   with backoff (0.5 s up to 4 s) until it answers, and then sends the request
   again, for up to 90 seconds (`deploy/cloudflare/src/hold.js`). A read (GET,
   HEAD, OPTIONS) is always sent again. A write is sent again only when the
   runtime said it never reached a server, and only with a body of known length
   up to 1 MiB, which the Worker copies; any other write gets the `503` at once,
   because sending it twice could do it twice. Reconnecting streams are reads,
   so they wait for the new server too.

A client therefore sees a slow answer, not an error, as long as the
replacement takes under 90 seconds. A cold start against a waking Neon is
included in that.

Two versions never serve at once: the old process has exited before the new
one boots and migrates, which is the cutover case in
[Migrations](Migrations.md). A blue-green deploy, with the new container
healthy before the old one stops, needs a second running instance (prod's
`max_instances` is 1 by decision, `deploy/cloudflare/test/cost.test.js`) or the
`durable_object` scheduling policy, which is in public beta, cannot be set on
the existing application, and does not work with the `Container` class this
Worker uses. If it is adopted later, the expand and contract rules in
Migrations apply to the overlap.

#### Measuring it

`scripts/measure-deploy-gap.sh` asks `/health` once a second, one request at a
time, and counts a request answered when the body names a version:

```sh
scripts/measure-deploy-gap.sh --origin https://dev.maidan.world --until-version dev-<first 8 of the commit>
```

Start it before the deploy (or the restart; omit `--until-version` and pass
`--for 300`). It stops once the new version has answered five times in a row and
prints `unanswered` (requests nothing answered), `longest_outage_s`,
`slowest_answer_s` (the longest a request was held), `longest_gap_s` and the
versions it saw. No downtime is `unanswered=0`; `slowest_answer_s` is the
time the replacement took. Both deploy workflows run it beside the deploy and
put its summary on the job's page, so every deploy records its number.

### Secrets

The Worker secrets are `DATABASE_URL`, `MAIDAN_CONTENT_KEK`,
`MAIDAN_SESSION_SECRET`, `MAIDAN_OIDC_CLIENT_SECRET`, `S3_ENDPOINT`,
`S3_ACCESS_KEY_ID` and `S3_SECRET_ACCESS_KEY`, set with
`npx wrangler secret put <NAME> -c wrangler.prod.jsonc`. The KEK, the session
secret and the OIDC secret are prod's own. The R2 keys should be a token scoped
to `maidan-artifacts-prod` alone.

`FEDERATION_ENCRYPTION_KEY` (32 random bytes, hex) encrypts webhook secrets, federation peer tokens, A2A push credentials and workspace secrets at rest. Without it, creating any of them answers 500. Keep it. To rotate it, set the new value as `FEDERATION_ENCRYPTION_KEY` and the old value as `FEDERATION_DECRYPT_KEYS` (comma-separated if more than one old key). The Worker forwards both.

`MAIDAN_EXPORT_SIGNING_KEY` (32 random bytes, hex) signs workspace exports.
Without it `GET /workspaces/{id}/export` refuses, and the support page's
export promise fails. `openssl rand -hex 32 | npx wrangler secret put
MAIDAN_EXPORT_SIGNING_KEY -c wrangler.prod.jsonc` sets it without printing it.
To rotate it, put the previous public key in `MAIDAN_EXPORT_VERIFY_KEYS` so
bundles signed by the old key still verify. `MAIDAN_CONTENT_KEK_PREVIOUS`
does the same for message text when `MAIDAN_CONTENT_KEK` changes. The Worker
forwards those companions.

Web push takes three more, `MAIDAN_VAPID_PRIVATE_KEY`,
`MAIDAN_VAPID_PUBLIC_KEY` and `MAIDAN_VAPID_SUBJECT` (an `https:` contact);
without them "This device" in the console cannot turn notifications on.
Result delivery to GitHub takes `MAIDAN_GITHUB_TOKEN` and
`MAIDAN_GITHUB_WRITE_REPOS`, both secrets so the repository list stays out of
this tree. Without them a session's change is never committed and a review
never reaches its pull request: the result is stored and its delivery fails.
A GitHub App can stand in for the token (`MAIDAN_GITHUB_APP_ID`,
`MAIDAN_GITHUB_APP_INSTALLATION_ID`, `MAIDAN_GITHUB_APP_PRIVATE_KEY`).
Result delivery to Slack takes `MAIDAN_SLACK_BOT_TOKEN`, and to Linear
`MAIDAN_LINEAR_API_KEY`. The Worker forwards each of these to the container
when it is bound. A result aimed at a surface whose credential is missing is
recorded `failed`, and its `last_error` names the setting.

## dev.maidan.world

Every push to `dev` that touches the server runs `deploy dev`
(`.github/workflows/deploy-dev.yml`). It builds the linux/amd64 image with a
warm cache, pushes it to the account registry, points `wrangler.jsonc` at it
and deploys, then waits until `/health` reports the build it deployed
(`dev-<first 8 of the commit>`). The job reads `CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID` from the `dev` environment; the token is scoped to
Workers scripts, containers and the `maidan.world` zone's routes. A pull
request that changes the deploy builds the image only.

The first deploy of a new instance creates the Durable Object and the
container application together, and the application can fail to find the
namespace (`DURABLE_OBJECT_NOT_FOUND`). The second attempt finds it, so the job
retries once.

Without local Docker, push a CI-built image with `crane`: download the
`maidan-server-image` artifact, get a short-lived push credential from
`POST /accounts/<id>/containers/registries/registry.cloudflare.com/credentials`
(`{"expiration_minutes": 30, "permissions": ["push", "pull"]}`), `crane push`
the tarball to `registry.cloudflare.com/<account id>/maidan-server:<tag>`, and
deploy with that reference as `image`. From a checkout, `npx wrangler deploy`
in `deploy/cloudflare/` builds the image from source with local Docker
instead, which is slow on a machine that is not linux/amd64.

Its secrets are set once and kept across deploys:

```sh
cd deploy/cloudflare
for name in DATABASE_URL MAIDAN_CONTENT_KEK MAIDAN_SESSION_SECRET \
            FEDERATION_ENCRYPTION_KEY MAIDAN_EXPORT_SIGNING_KEY \
            S3_ENDPOINT S3_ACCESS_KEY_ID S3_SECRET_ACCESS_KEY; do
  npx wrangler secret put "$name"
done
```

`FEDERATION_ENCRYPTION_KEY` is what lets this instance register a webhook, a
federation peer, a workspace secret, or an A2A push config. Without it those
calls answer 500. `MAIDAN_EXPORT_SIGNING_KEY` signs `GET /workspaces/{id}/export`.
When either key, or `MAIDAN_CONTENT_KEK`, is rotated, set the previous value
too: `FEDERATION_DECRYPT_KEYS`, `MAIDAN_EXPORT_VERIFY_KEYS` (the old public
key), and `MAIDAN_CONTENT_KEK_PREVIOUS`. The Worker forwards all three.

The Worker passes only the names it lists in `src/server-env.js` to the container,
plus the fixed settings there (`MAIDAN_ENV=production`, JSON logs, the bind
address and one trusted proxy hop). Every value is a string, so a switch set
to `1` arrives as `1` (the server also accepts `true`). A running container
keeps the environment of the `start()` that created it, and an image rollout
can leave the new image on the previous start's environment: when the values
change, the Worker recycles the container before the next request. Plain
settings (the public origin, the bucket, dynamic client registration) are
`vars` in `wrangler.jsonc`.

## Internal hosts

The server can send webhooks to an operator's own Workers without either side
having a public API: a Worker woken by a webhook needs no queue-depth poll,
and one that serves only through a binding keeps its API off the internet
entirely. The request goes from the
container to the instance's Worker and on to the other Worker over a service
binding, and never crosses the public internet.

Three settings, all in the instance's Wrangler config:

1. **A binding.** `services` names the Worker, and for a Worker that keeps its
   API off its public `fetch`, the named `WorkerEntrypoint`:
   `{ "binding": "HOOKS", "service": "<the receiving Worker>", "entrypoint": "Internal" }`.
2. **The outbound map.** `MAIDAN_INTERNAL_HOSTS` maps a hostname under
   `.internal` to that binding (`hooks.internal=HOOKS`). The Worker intercepts
   the container's plain-HTTP calls to each mapped name (`outboundByHost` in
   `@cloudflare/containers`) and forwards them to `env[BINDING].fetch`. A
   name not under `.internal`, or one whose binding the config does not
   declare, is not mapped. Every other host keeps the container's own internet
   egress, which the server needs for Neon, R2, GitHub and Slack.
3. **The egress list.** `MAIDAN_INTERNAL_EGRESS_HOSTS` names, exactly, the
   `.internal` hosts the server may send to (`hooks.internal`). The egress
   guard refuses every other `.internal` name when a webhook is registered and
   again when it is delivered. It does not check a listed name's address,
   because the container network answers the name with a placeholder that
   routes to the Worker. Nothing else gets looser: plain HTTP to a public host
   keeps the public-address rule, and private addresses still need
   `MAIDAN_PRIVATE_EGRESS_HOSTS`. The Worker hands the server only the listed
   names it also maps to a declared binding, so a name whose route is missing
   is refused rather than sent to the internet.

The list covers the whole instance, so on an instance with customer workspaces
any workspace admin could point a webhook at a listed host. The receiving
Worker must therefore check `X-Maidan-Signature` against the subscription's
secret, as any webhook receiver must.

With `HOOKS` declared and `hooks.internal` mapped and listed, a webhook on
ready threads wakes that Worker:

```sh
curl -X POST https://<instance>/workspaces/$WS/webhooks \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"url":"http://hooks.internal/wake","label":"wake the hooks Worker","event_kinds":["thread_ready"]}'
```

The configs in this repository (prod, dev, the demo and the docs) declare none
of this; an operator adds it to their own instance's config.

## The first admin

`maidan init` writes through the store, so it runs from any machine that can
reach Neon, with the same `DATABASE_URL` and `MAIDAN_CONTENT_KEK` as the
instance's Worker secrets ([First admin and tokens](Production.md#first-admin-and-tokens)).

## Public status page

The Worker serves `/status` to anyone: whether the database, artifact storage,
live updates and search index are working, read from the container's
`/health`, or a plain "not answering" when the container does not reply within
eight seconds. It never shows the server's error text. One reading answers the
page for a minute, so a crawler cannot keep a sleeping container awake.

## What a sleeping instance means

- The first request after a sleep waits for the container to start and the
  server to boot against Neon, which may itself be waking.
- Any open request keeps the container awake: a WebSocket, an SSE stream or
  MCP's notification stream counts until it closes, and the container sleeps
  15 minutes after the last one does. An agent that stays connected keeps its
  instance running, and is billed as running.
- Work a sleeping server would have done waits for the next request or the
  daily cron (09:17 UTC). Each wake costs 15 minutes of container and Neon
  compute, so the cron is daily on purpose: cost comes first while the
  instance is mostly idle.
- Any request wakes it, a crawler's too. Scanners find a new domain within
  hours of its certificate and probe for `/.env`, `/.git/config` and PHP
  files. The Worker answers paths no Maidan route serves with 404 itself
  (`src/scanner.js`), so those probes never reach the container.

## The docs site

`docs.maidan.world` serves the docs on the domain layout (Decisions,
2026-10-12). It is the mdBook site as static assets on its own Worker with no
code, `maidan-docs` (`deploy/cloudflare/wrangler.docs.jsonc`), built with
`site-url` `/` and the response headers in `deploy/cloudflare/docs-headers`.
GitHub Pages keeps serving the same site at `/maidan/` until the domain is
live.

The `docs.maidan.world` job in `.github/workflows/docs.yml` builds and deploys
it on every push to `main`, but only while the repository variable
`DOCS_DOMAIN` is `true`. To turn it on:

1. Add `docs.maidan.world` to the `maidan.world` zone. The custom-domain route
   in the config creates the DNS record and certificate on the first deploy.
2. Set the repository variable `DOCS_DOMAIN` to `true` and run the docs
   workflow on `main`.
3. Check `https://docs.maidan.world/` answers with HSTS, then point the
   README's and the landing page's docs links at it.
