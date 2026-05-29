# Cluster 37.0 retro — A2A SendStreamingMessage

> Tag **`v37.0.0`**.

## What shipped

- `SendStreamingMessage` JSON-RPC method on `POST /a2a/v1/rpc` returns `text/event-stream`.
- Each SSE `data` frame is a JSON-RPC 2.0 response with a `StreamResponse` result (`task`, then `statusUpdate`).
- Shared message-post path with `SendMessage`; `A2aClient::send_streaming_message` parses the SSE stream.
- E2E: working task frame followed by completed `TaskStatusUpdateEvent` with `final: true`.

## What was deferred

- `SubscribeToTask` resubscription after disconnect.
- `TaskArtifactUpdateEvent` chunked artifacts.
- Push notification configs.

## Forward look

Cluster **38**: MCP resource notifications on HTTP edit, purge, vote, mention.
