### A lost MCP claim is tried again, not taken twice

- **Fixed:** The OpenRouter claimant sends one idempotency key and one request body for every retry of a call, and stops a call that has dripped past its time limit. A claim the server already took comes back on the retry instead of taking a second thread.
