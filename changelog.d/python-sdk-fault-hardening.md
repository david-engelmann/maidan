### A quiet subscription stays open, and a slow answer cannot hold a call

- **Fixed:** The Python client keeps a WebSocket subscription alive when the room is quiet, refuses a handshake that is not a real 101, and stops an HTTP call that drips past its time limit. A retry of a write sends the same bytes, and a `Retry-After` that is not a number, or is an HTTP date, is read correctly.
