# Cluster 347.0 retro — projector egress wire-path tests (audit P1.5)

> Tag **`v347.0.0`**. Phase XXIV (post-gate hardening). **Cluster 16 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The audit's P1.5 test-confidence gap on a shipped launch feature. The production HTTP clients that
build the *actual* outbound projector-egress request — `SlackWebClient` (`chat.postMessage`) and
`GithubApiClient` (issue-comment POST) — were never exercised. The existing projector-egress tests
drive the mock `SlackSender`/`GithubSender` traits, so the exact URL, headers, body, and
success/error decoding of the real wire request were unverified. Now that Cluster 346 made the
egress operable, this hardens it:

- Each client gained a **`with_base_url`** constructor (production `new` delegates to it with the
  real host), so the wire path can be pointed at a loopback server.
- **`egress_wire_e2e`** drives the real clients against a recording server:
  - Slack posts `POST /api/chat.postMessage` with `{channel, text}` + `Authorization: Bearer …`,
    and decodes Slack's HTTP-200-`{"ok": false, "error": …}` convention to `SlackError::Api`.
  - GitHub posts `POST /repos/{repo}/issues/{n}/comments` with `{body}` + bearer + the required
    `User-Agent`, and maps a non-2xx to `GithubError::Api(status)`.

## Surprises / decisions

- **Base-URL override, not a mock-transport swap.** The point is to test the *real* client
  (reqwest request-building, header set, JSON shape, response decoding) — a trait mock would test
  none of that. A minimal `with_base_url` is the smallest seam that makes the real client
  testable, and production behaviour is unchanged (`new` still targets the real host).
- **The `User-Agent` assertion is load-bearing.** GitHub rejects a request with no `User-Agent`;
  the test pins that the client sets `maidan-projector`, so a refactor can't silently drop it.
- **SMTP left as-is.** The mail transport already has a recording-mock e2e (Cluster 249) + a
  connect-free config-validation unit test; a full in-process SMTP catcher is lower-value and is
  logged as an optional follow-up. The LSN-replica CI job (the other P1.5 half) is deferred to its
  own cluster — it needs a two-Postgres Docker setup in CI (heavy), while the routing is already
  validated locally against `scripts/replica-harness.sh`.

## Test evidence

`egress_wire_e2e` (4 cases: Slack ok + ok-false-error, GitHub created + non-success). fmt + strict
clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Remaining audit items: the **LSN-replica CI job** (P1.5 second half); P2 code-side — notification
batch insert (Cluster-344 follow-up) and the Store trait split. Optional: SMTP wire test, MCP
projector-link tools.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
