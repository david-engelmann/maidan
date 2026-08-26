# Cluster 279.0 retro — `maidan init`

> Tag **`v279.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P0:
> production-safe first-admin bootstrap.** No new gate tag.

## What shipped

A one-time `maidan init` CLI command that removes the "you need an admin token to
create the first admin token" chicken-and-egg without `AUTH_DISABLED` or public
bootstrap HTTP routes:

```sh
DATABASE_URL=postgres://… maidan init --workspace my-team --admin-handle david
```

- Connects to the store (SQLite or Postgres), runs migrations, then creates the first
  workspace and an admin member via the `create_*_with_event` store methods (so the
  `WorkspaceCreated`/`MemberJoined` events land in the log exactly as if the server had
  created them), and mints an **all-capabilities** bearer token.
- Prints the token **once** to stdout (logs go to stderr), then explains that it holds
  every capability and that narrower per-agent tokens should be minted from it.
- **Refuses if the store already has a workspace** (exit non-zero), so it can never
  clobber an existing deployment or mint a second root token.
- New `maidan_auth::capability::all()` exposes the previously-private `KNOWN` list as
  the superuser set.

The production image can stay bootstrap-stripped (`--no-default-features`): `init`
writes through the store, not the bootstrap routes.

## Surprises / decisions

- **The mint path is the server's, so auth is free.** `init` builds the token with the
  same `create_api_token(NewApiToken { token_hash: hash_secret(secret), … })` call the
  server's admin mint uses, so the printed token authenticates through the normal
  `resolve_bearer` path (already covered by the auth e2es). Verified the stored row
  carries all 12 capabilities.
- **All-capabilities, on purpose.** The first token is the bootstrap superuser; the
  operator scopes down from it. Added `capability::all()` rather than hardcoding a list
  in the CLI, so it stays in sync with the capability set.
- **Refuse, don't force.** A first-admin bootstrap that could run twice is a footgun
  (a second root token, or clobbering state). `count_workspaces() > 0` → hard refusal,
  no `--force`. If you need another workspace or token, use the API with the first token.
- Reused the SQLite single-connection default (Cluster 277) for the CLI pool too, so the
  three sequential writes (workspace, member, token) can't self-contend.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan init` subcommand | `crates/maidan-cli/src/main.rs` |
| `capability::all()` superuser set | `crates/maidan-auth/src/capability.rs` |
| Production.md bootstrap section (init as the recommended path) | `docs/Production.md` |
| bootstrap-once / refuse-twice integration test | `crates/maidan-cli/tests/init.rs` |

## Risks identified + still open

- `init` prints the token to stdout; an operator who loses it must mint a new one via
  the first token (or, if that is lost too and no other `token:admin` exists, re-seed a
  fresh database). That is the intended "shown once" security posture.
- The quickstart (Cluster 278) still uses the dev auth-disabled path for its demo; `init`
  is the path for real deployments.

## Forward look

Two launch-readiness items remain after this: **LangChain/AutoGen recipes + interop CI**,
then a **published benchmark**, then the larger **A2A v1.0 compliance** arc.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 278.0]]. Finding from the external launch-readiness review (Cluster 274).
