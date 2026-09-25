# Goose as a Maidan worker

[`maidan-waiter.yaml`](maidan-waiter.yaml) is a [Goose](https://github.com/aaif-goose/goose)
recipe. Each run takes one task off a Maidan channel, does it with Goose's own
tools, and hands the result back. Maidan runs the waiter loop around the work
(claim, acknowledge, renew, result, review, release); the task text is whatever a
person or another agent filed on the channel.

## Run it

1. Give Goose its own identity: install it as an app with a token that holds
   `workspace:read`, `message:post` and `thread:transition`
   ([Integration, minimal integration step 3](../../docs/Integration.md#minimal-integration-http)).
   Do not give it the admin token.
2. Point the recipe's `uri` at your server's `/mcp/streamable` if it is not
   `http://127.0.0.1:8080`.
3. Run it:

```sh
export MAIDAN_TOKEN=maid_...     # read by the recipe's env_keys; keep it out of the file
goose run --recipe examples/goose/maidan-waiter.yaml \
  --params channel_id=<channel uuid> --no-session
```

A run with no work says "no work" and exits. To keep a worker on the channel:

```sh
while true; do goose run --recipe examples/goose/maidan-waiter.yaml \
  --params channel_id=<channel uuid> --no-session; sleep 10; done
```

## What the recipe asks of Goose

The instructions spell out the loop in eight steps, using ten Maidan tools. The
two that matter most:

- **Hand the thread to review, then release it.** Only `open` threads are handed
  out, so `transition_thread start_review` is what keeps finished work from
  coming back. Goose never closes a thread; a reviewer does.
- **Ask before acting irreversibly.** For a deploy, a deletion or a spend, the
  recipe has Goose open an approval gate on the thread and wait. It goes ahead
  only on `accepted`, and a person answers in `/ui`. Maidan refuses an approval
  from whoever requested it.

Goose's streamable HTTP client opens with MCP `2026-07-28`, which Maidan speaks,
so the legacy-server fallback bug in Goose 1.51
([aaif-goose/goose#12257](https://github.com/aaif-goose/goose/issues/12257)) does
not apply.
