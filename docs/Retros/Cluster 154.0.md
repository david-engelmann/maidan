# Cluster 154.0 retro — `request_client` GET-stream delivery fix

> Tag **`v154.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Lane 3 (of the three-lane plan), part 1 — the transport correctness fix.

## What shipped

- **A per-session server→client-request broadcast.** `streamable_session` gains
  `client_req_tx` (a `broadcast::Sender<String>`), `push_client_request` (send +
  `is_ok()` — did it reach ≥1 open GET leg, so `request_client` can still fail
  fast when nobody listens), and `subscribe_client_requests`.
- **`request_client` delivers on it** instead of the POST-leg mpsc.
- **`stream_get` merges it** with the unsolicited notifications, so the
  canonical `GET /mcp/streamable` stream now carries server→client requests.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Cluster 155 | A real caller | Needs session context threaded into tool dispatch; then a sampling-backed `summarize_thread`. |
| Future | `Last-Event-ID` replay of server→client *requests* | Requests are live-only to a connected GET leg; a disconnected client re-drives the request on reconnect. Notification replay unchanged. |

## Surprises

- **Two channels, by design, not one.** The instinct was to make the session's
  single mpsc multi-consumer (broadcast) so both legs share everything. But the
  POST leg's *response* must not fan out to GET legs, and the GET stream must not
  replay the POST leg's response. Keeping the response/notification mpsc separate
  from a new request-only broadcast is both lower-risk (Clusters 129/147 paths
  untouched) and more correct — responses and server-initiated requests belong
  on different legs per the MCP spec.

## Decisions

- **Deliver server→client requests GET-only** (not dual POST+GET). Dual delivery
  would duplicate a request to a client holding both legs; GET-only matches the
  spec (the GET stream *is* the server→client channel) and there is no organic
  caller to regress. The Cluster 148 round-trip e2e was updated to read from a
  GET stream — a better test as a side effect.
- **`push_client_request` returns "reached a live leg"** so `request_client`
  keeps its fail-fast contract (no listener → error, not a 30 s timeout).

## Capability table extension

| Capability | Where |
|------------|-------|
| Server→client requests on the canonical GET stream | `streamable_session.rs`, `mcp_streamable.rs` |

## Risks identified + still open

- **Low–moderate (transport).** Behavior change: a POST-only client no longer
  receives server→client requests. Mitigated by: no organic caller yet; the
  POST-leg response/notification/replay paths are untouched; unit + e2e cover
  the new path and the fail-fast case.

## Forward look

Lane 3 part 2 (Cluster 155) gives `request_client` a real caller: thread an
optional session id through `handle → dispatch → tools_call → tools::dispatch`,
then a sampling-backed `summarize_thread` tool. Its e2e — spawn the `tools/call`,
read the sampling request off the GET stream, POST the client's summary — will
exercise this cluster's delivery path end-to-end and close the three-lane plan.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
