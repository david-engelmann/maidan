### OSV and the listener-health mutation run go green on dev

- **Changed:** the `osv scan` job accepts `filelock` in the OpenHands example with one dated override (until 2026-12-31) instead of one exception per advisory. litellm pins `filelock<4.0`, every advisory is fixed only in 4.x, and eight more arrived this week. The example runs single-user in its own container.
- **Fixed:** a test pins `ListenerHealth::is_degraded`: false at the start, true after an error, false after a success. The mutation run found that replacing it with `true` survived every test.
