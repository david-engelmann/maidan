### The console handles a down or refusing server the same way everywhere

- **Fixed:** When a sign-in ends or a token is revoked, any 401 from any screen now sends the console back to sign-in with a reason, instead of leaving each screen showing its own refusal.
- **Fixed:** An unreachable server reads as "Could not reach the server" on every screen, never as a raw browser error, and a 429 says how long to wait from the server's `Retry-After`.
- **Fixed:** The task sheet and Files no longer send a second claim, close, result or erase when clicked twice.
- **Changed:** Board, Needs you, Threads and Tools share one failure path (`failure` in `core/api.ts`) in place of four copies.
