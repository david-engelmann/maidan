### The A2A client keeps streaming text intact and refuses a cleartext token

- **Fixed:** `A2aClient` reassembles SSE frames from raw bytes, including a character split across chunks, `\r\n\r\n` delimiters, and a final event with no trailing blank line. An event with no delimiter is refused once it passes 16 MiB.
- **Fixed:** The client times out a dead socket, does not follow redirects, sends a Maidan user agent, numbers each JSON-RPC call, and checks that a success response echoes that id. A non-2xx response includes a short snippet of the body. A bearer token is not sent to a cleartext host outside this machine.
- **Fixed:** A federation peer URL must name a host and must not carry userinfo, a query, or a fragment. A non-2xx pull or push includes a short snippet of the remote body.
