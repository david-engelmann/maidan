### The Go client's subscription names the host the way a browser does

- **Fixed:** The Go client's WebSocket handshake sends `Host` without the port when the port is the scheme's default (443 for `wss`, 80 for `ws`), so an edge that routes by host name accepts the subscription.
