# Hosted console (design)

The design note for the hosted console's first version, written 2026-10-09
against `main` at `58e942f3`. It unblocks the workspace
switcher, and says what the rest of the console needs before a hosted instance
can take sign-ups. Every claim about today's behaviour names the file it comes
from. Anything this note proposes is marked **Proposed**, and anything that
doesn't exist yet is marked **Not built**.

The connected-apps entry in [Decisions](Decisions.md) (2026-10-03, with the
maintainer's 2026-10-04 rulings) holds: hosting is hybrid and open source
first, self-hosting is the product, and "the hybrid hosting decision does not
limit the hosted console". Nothing here changes what a self-hosted deployment
does unless its operator turns it on.

## What exists today

These rows describe the code, not product claims. None of them is a new claim
for [Claims](Claims.md), and this note adds none. Where the code falls short,
the note says so as a gap (open question 6) rather than claiming the
behaviour.

| Piece | Where | What it does |
| --- | --- | --- |
| OIDC sign-in | `crates/maidan-server/src/oidc/handlers.rs` | `GET /auth/oidc/login?workspace_id=…` stores a one-time `state`, a nonce and a PKCE verifier bound to that workspace (`maidan_oidc_pending`), then redirects to the provider. `GET /auth/oidc/callback` checks the ID token's signature, nonce and issuer, resolves a member, and creates a server-side session (`session.create`, audited). |
| Identity to member | `crates/maidan-server/src/oidc/member.rs` | `resolve_member_for_login` looks up `(workspace, issuer, subject)` in `maidan_oidc_identities`. Failing that, it links a pre-provisioned member whose handle equals a verified email (`MAIDAN_OIDC_LINK_EMAIL`). Failing that, it creates a human member (`MAIDAN_OIDC_AUTO_PROVISION`). Otherwise it refuses with "not provisioned in this workspace". |
| Identity rows | `migrations/*/0012_oidc_sessions.sql` | `maidan_oidc_identities` is unique on `(workspace_id, issuer, subject)`. The same person has one row per workspace they have signed in to, each pointing at that workspace's member. |
| Sessions | `crates/maidan-server/src/session/` | A `maidan_sessions` row has a workspace, a member and, for a session made from a token, the token's id. The signed `maidan_session` cookie names the row. `SessionContext` carries `session_id`, `member_id`, `workspace_id` and the token's authority, if any. Nothing on the row says which identity signed in. |
| Session authority | `crates/maidan-server/src/auth.rs` | An OIDC session gets fixed capabilities on the `/ui/api` routes: `OIDC_READ_CAPABILITIES` (read, subscribe, search) and `OIDC_WRITE_CAPABILITIES` (adds write, post, transition, upload). It never holds `token:admin`. A session made from a token holds exactly that token's authority. |
| First admin | `crates/maidan-server/src/session/handlers.rs` | `POST /auth/session/mint` lets an OIDC session mint the workspace's first `token:admin`, only while none is live and `MAIDAN_OIDC_FIRST_ADMIN` allows it. |
| Console sign-in | `crates/maidan-server/static/ui/main.js` | The **Sign in** button needs the workspace id typed in first ("Enter the workspace ID first.") and sends it to `/auth/oidc/login`. |
| A second workspace | `crates/maidan-server/src/routes/workspace.rs` | `POST /operator/workspaces` (`operator:global`, #1208) creates a workspace, its first human admin and a one-time admin token in one transaction, without `MAIDAN_BOOTSTRAP`. |
| Connect an agent | `crates/maidan-server/static/ui/main.js` | The sheet (#1144) creates an agent member with `POST /workspaces/{wid}/members` and mints a `maidan.agent.worker` token with `POST /workspaces/{wid}/members/{mid}/tokens` (`token:admin`). |
| SCIM | `crates/maidan-server/src/scim.rs` | `token:admin` creates and updates human members at `/scim/v2/Users`. `active: false` revokes the member's tokens and releases its claims. |

[OIDC](OIDC.md) has the whole sign-in flow and its configuration. The
Keycloak recipe there (#1320) is the tested end-to-end path: SCIM creates the
member, and link-by-verified-email binds the identity on first sign-in.

## Sign-up through the existing OIDC provider

**Not built.** There is no sign-up. A person signs in to a workspace that
already exists, using its id. Every workspace today comes from `maidan init`,
the bootstrap route, or `POST /operator/workspaces`.

**Proposed.** Sign-up uses the instance's one OIDC provider and adds no
identity system, no password store and no second provider configuration. It
is a variant of the existing login:

1. **Starting it.** `GET /auth/oidc/signup` starts the same code-and-PKCE flow
   with no workspace. That needs `maidan_oidc_pending.workspace_id` to become
   nullable (a migration) and a pending-row kind, so a sign-up `state` can
   never complete a sign-in or the other way round.
2. **The callback.** It requires `email_verified`. It then creates, in one
   transaction:
   - the workspace;
   - a human admin member;
   - the identity row;
   - the session.

   This is the same shape as `POST /operator/workspaces`, whose PR found that
   "creating an empty workspace would still leave nobody who can sign in"
   (#1208). The rows are audited as `workspace.signup` and `session.create`.
3. **The first admin token.** The new member has no token. It mints its first
   `token:admin` with the existing `POST /auth/session/mint`, so sign-up adds
   no new minting path.
4. **Off by default.** It runs only with `MAIDAN_SIGNUP=1`. A self-hosted
   deployment that never sets it is unchanged. A hosted instance can also set
   `MAIDAN_SIGNUP_EMAIL_DOMAINS` to limit who may sign up.

`MAIDAN_OIDC_AUTO_PROVISION` must stay off on a hosted instance. With it on,
anyone the provider authenticates who knows a workspace id becomes a human
member of that workspace on first sign-in (`resolve_member_for_login`). That
is right for a single-company deployment and wrong for a shared instance. The
hosted configuration is:
- `MAIDAN_OIDC_AUTO_PROVISION` unset;
- `MAIDAN_OIDC_LINK_EMAIL=1`;
- `MAIDAN_SIGNUP=1`.

**Proposed:** boot refuses `MAIDAN_SIGNUP=1` together with auto-provisioning.

## One identity, several workspaces

A person is an `(issuer, subject)` pair at the provider. In Maidan they are a
separate member in each workspace, with one identity row per workspace. The
rows share the issuer and subject and point at different members. That is
deliberate: [OIDC](OIDC.md) lists "No automatic cross-workspace identity —
workspace remains the tenancy boundary" as a linking rule, and this note
keeps it.

- **Becoming a member of a second workspace.** That workspace's admin creates
  the member, over SCIM or the console. The person's first sign-in to it links
  the identity by verified email. Nothing creates a member in one workspace
  because the person is a member of another.
- **What a session is.** A session stays one member in one workspace, with
  that workspace's capabilities. A person in three workspaces has three
  members, and the browser carries one session cookie at a time. Today an
  earlier session row stays live on the server until it expires, even after
  the cookie is replaced. The switcher ends it (see "Switching").
- **What the session carries.** Today it is `(workspace, member, token?)`.
  **Proposed:** add the signed-in identity, `maidan_sessions.oidc_identity_id`
  (nullable, `ON DELETE CASCADE`), set by the OIDC callback and `NULL` for a
  session made from a token.

  The member alone is not enough. A member can hold two identities: link by
  email attaches a second `subject` with the same verified email to the member
  the first one created. Keying on the member would then list the second
  subject's workspaces to the first. Keying on the identity the person
  actually signed in with lists exactly theirs. Deleting the identity row ends
  the session.
- **What a person is not.** There is no person or account row above members,
  and this note adds none. "Multi-tenant orgs above workspace" stays a non-goal
  ([OIDC](OIDC.md), Non-goals). The identity rows already are the join.

## The agent invite (#1144)

#1144 made **Connect an agent** finishable:
- it creates an agent member;
- it mints `maidan.agent.worker`, which can claim, post and transition;
- it shows the secret once and never writes the browser's own token into the
  snippet.

On a hosted instance it doesn't work, in two places.

- **Creating the member uses the bootstrap route.** `POST
  /workspaces/{wid}/members` is in the bootstrap router
  (`crates/maidan-server/src/app.rs`, `#[cfg(feature = "bootstrap")]`). It
  answers only with `MAIDAN_BOOTSTRAP=1` or auth disabled
  (`crates/maidan-server/src/bootstrap.rs`). The production image is built
  with `--no-default-features` (`crates/maidan-server/Dockerfile`), so the
  route isn't there at all. #1144's retro deferred "an authenticated
  create-member route for servers that have turned bootstrap off". SCIM
  creates human members only.
- **Minting needs `token:admin`.** An OIDC session never holds it. A person
  who signed up has it only after `POST /auth/session/mint`, and then only as
  a token the page exchanges for a session.

**Proposed (not built):** `POST /workspaces/{wid}/agents` (`token:admin`). It
creates an agent member and its `maidan.agent.worker` token in one
transaction, audited as `member.create` and `token.mint`, and returns the
secret once. The route never creates a human, never takes a capability list,
and is mounted on the production image. Connect an agent calls it instead of
the bootstrap route and falls back to the current two calls only where the
bootstrap route exists. An "invite" is then this secret, handed to the agent's
operator out of band, as today.

## What #1208 allows

`POST /operator/workspaces` (`crates/maidan-server/src/routes/workspace.rs`,
#1208):
- **Who can call it:** a holder of `operator:global` opens a workspace on a
  running instance, with no `MAIDAN_BOOTSTRAP`.
- **What it creates:** the workspace, a human admin member, and an admin
  token holding every capability except `operator:global` and
  `audit:read-global`.
- **What that token can't do:** a later mint cannot add those two unless the
  caller already holds them, so a tenant admin can never become an instance
  operator.
- **Where it runs:** the route is on the production image.
- **Docs:** [Production](Production.md), "A second workspace".

It is the authorization change the hosted console needs, not the console:
- the operator calls it with a bearer;
- nothing binds the new admin to an OIDC identity;
- the admin token is the only way in until that person signs in. With link by
  email on, they can, if the admin handle is their verified email.

Sign-up (above) is this route's transaction, run by the callback for the
signed-in person instead of by an operator.

## What stays self-hosted only

These need an operator, not a tenant, and a hosted instance's tenants never
get them:

| Capability or setting | Why it stays with the operator |
| --- | --- |
| `operator:global`, `audit:read-global` | They read or act across tenants (`tenant_admin_capabilities` excludes both). |
| `maidan init`, `MAIDAN_BOOTSTRAP`, `AUTH_DISABLED` | One-shot setup and test modes. The production image has no bootstrap routes. |
| The OIDC provider and its settings (`MAIDAN_OIDC_*`) | One issuer and client per process (`crates/maidan-server/src/oidc/config.rs`). A tenant cannot bring its own provider on a shared instance. **Not built:** per-workspace providers. |
| Instance limits, retention defaults, read replicas, backups, signing and encryption keys | Environment and deployment settings ([Production](Production.md)). A tenant sets only its own workspace's retention and policies. |
| Federation peers, outbound egress (GitHub App, Slack), secret broker | Configured per process. A hosted instance would offer them per workspace only after its own design. |
| SCIM from the tenant's own directory | Works per workspace (`token:admin`) on any instance, but a hosted tenant's directory points at the shared provider's users, so it is only useful self-hosted until per-workspace providers exist. |

## The workspace switcher

**Not built.** The rest of this section is the design the switcher builds.

### Listing

**Proposed:** `GET /auth/session/workspaces`, on the session-only tree beside
`GET /auth/session` (`crates/maidan-server/src/app.rs`, `auth_routes`, behind
`session::require_middleware`). A bearer gets `401`, as on `/auth/session`.

- **What it returns:** every workspace in which the session's signed-in
  identity has an identity row, each as `{ workspace_id, name, member_id,
  handle, current }`, ordered by the identity row's `last_login_at`, newest
  first.
- **How it finds them:** identity rows are per workspace, so comparing row
  ids would only ever find the current one. The lookup has two steps. First,
  resolve the session's `oidc_identity_id` to that row's `(issuer, subject)`.
  Then select every `maidan_oidc_identities` row with that same issuer and
  subject, in any workspace, joined to its workspace and member.
- **A session made from a token:** only its own workspace (`current: true`).
  The token is one workspace's credential and proves nothing about the person
  behind it.
- **A deactivated member is left out.** A workspace whose member is SCIM
  `active: false` (`maidan_scim_users.active = 0`) is excluded. So is a frozen
  member's (`maidan_member_freezes`), except the current one.
- **Nothing beyond the person's own workspaces:** no counts, no other
  members, no workspace the identity has no row in.
- **Bounded at 200 rows.** The name search runs in the console over this list.
  There is no server-side search parameter, so the route cannot be used to
  probe names.

### Switching

**Proposed:** a switch is a fresh sign-in to the target workspace. The console
sends the browser to `GET
/auth/oidc/login?workspace_id={target}&return_to=/ui/`, the route it already
uses (`static/ui/main.js`). With a live provider session, the provider often answers
without a prompt, and the person sees only a redirect. That isn't guaranteed:
the request doesn't set `prompt=none`, so the provider may still ask for a
login, consent or another step, depending on its own policy. The
callback then:
- resolves the member with `resolve_member_for_login`, as for any sign-in;
- creates the new session, with its `oidc_identity_id`;
- **ends the browser's previous session** (`session.delete`, reason
  `switched`), as `session_from_token` already does with reason `replaced`.
  Today the callback leaves the old row live until it expires.

No route mints a session for another workspace from the current one. That is
deliberate: the only thing that issues an OIDC session stays the callback,
after the provider has said who the person is. The checks it runs are the
issuer, the nonce, the one-time `state` bound to the workspace, and the
member lookup. A person whose provider account was disabled since their last
sign-in cannot switch.

### Tenant-isolation rules

1. **The listing never names a workspace the identity has no member in.** It
   is keyed on the session's `oidc_identity_id`, never on a handle, an email,
   or a workspace id the client sends.
2. **Switching to a workspace you aren't a member of gives no session.** The
   callback refuses unprovisioned identities (auto-provisioning off), and
   creates no member and no identity row.
3. **A token's session never sees past its token's workspace.**
4. **One live browser session per sign-in.** A switch ends the previous one.

### The two-tenant test it needs

`session_workspaces_e2e`, on the mock provider. It sets up:
- workspaces A, B and C;
- identity X with members in A and B;
- identity Y with a member in C and one in A.

It checks:
- **Listing:**
  - X signed in to A lists exactly {A, B}, never C;
  - Y signed in to C lists {C, A};
  - a session made from a token in A lists only {A};
  - after B's member is SCIM-deactivated, X lists only {A}.
- **Switching:**
  - X switching to B gets a session for X's B member, and the A session is gone;
  - X switching to C gets a `403` with no session cookie, and C gains no
    member and no identity row;
  - X's session cookie on Y's workspace id lists nothing of Y's.
- **Break-checks:**
  - drop the identity filter and the test sees C;
  - key on the member instead of the identity and a doubly-linked member
    leaks;
  - stop ending the previous session and the old cookie still works.

### The console

The board's header shows the current workspace's name. With two or more
workspaces listed it becomes a button that opens a search box (filtering by
name as you type) and the list. Choosing one navigates to the login URL above.
With one workspace, or a token session, nothing changes. A Playwright spec
drives it against the seeded harness
(`crates/maidan-server/examples/ui_test_server.rs`).

## Open questions

Each has a recommended answer. None blocks the switcher as designed above.

1. **Switch by re-sign-in, or by a session swap?**
   - **The question:** a swap route (`POST /auth/session/switch`) would skip
     the provider round trip.
   - **Recommended:** re-sign-in. It keeps one session-issuing path, picks up
     a provider-side disable, and costs a redirect the provider usually
     often answers without a prompt (not guaranteed: no `prompt=none`).
   - **Revisit if** a provider without silent SSO makes switching prompt
     every time.
2. **Store the identity on the session, or derive it from the member?**
   - **Recommended:** store it (`oidc_identity_id`, one migration on both
     backends). Deriving it from the member leaks across a member with two
     linked subjects.
   - **Old sessions:** sessions from before the migration list only their own
     workspace until the next sign-in.
3. **Should the listing include workspaces the person was invited to but has
   never signed in to?**
   - **The question:** a pre-provisioned member has no identity row until the
     first sign-in.
   - **Recommended:** not in v1. Matching by email across workspaces is the
     "automatic cross-workspace identity" [OIDC](OIDC.md) rules out. An
     invite link (a sign-in URL with the workspace id) covers it.
4. **A front door without a workspace id?**
   - **The question:** the console's Sign in needs the id typed.
   - **Recommended:** after the switcher, let `/auth/oidc/login` take no workspace
     id. It signs in to the identity's most recently used workspace, or
     offers sign-up when it has none and sign-up is on.
   - **Not in the switcher:** it needs the nullable pending row that sign-up needs.
5. **The pre-authentication existence oracle.**
   - **The question:** `/auth/oidc/login` answers an unknown workspace id
     with an error before redirecting (`get_workspace`). It tells anyone
     whether an id exists.
   - **Recommended:** keep it for now, since ids are random UUIDs. When the
     front door lands, redirect for unknown ids too and fail at the callback
     (the [Decisions](Decisions.md) entry on workspace handles weighs the
     same trade).
6. **Does SCIM deactivation end OIDC sign-in?**
   - **The answer today:** not as built. `scim_update_user_audited` revokes
     tokens and releases claims, and `resolve_member_for_login` doesn't read
     `maidan_scim_users.active`. A deactivated member can still sign in, and
     a live session keeps working until it expires.
   - **Recommended:** refuse sign-in for an inactive SCIM member and end that
     member's sessions on deactivation. It's a separate fix with its own
     test, and the switcher's listing already leaves such workspaces out.
   - **Status:** the maintainer's review on #1345 treats this as a live bug,
     not an open question, and it is being fixed in its own PR.
7. **Where does OAuth consent happen for a person with several workspaces?**
   - **The answer:** consent is in the console (docs/OAuth.md, decided
     2026-10-08, arriving with #1324). The grant belongs to the session's
     workspace.
   - **Recommended:** the consent page names that workspace and offers the
     switcher. This note says nothing else about the authorization server.

## Order of work

1. The workspace switcher (listing, switch by re-sign-in, `oidc_identity_id`,
   the console control, the two-tenant test).
2. The SCIM-deactivation sign-in fix (open question 6).
3. `POST /workspaces/{wid}/agents`, the invite on a production image.
4. Sign-up behind `MAIDAN_SIGNUP`, and the front door without a workspace id.
