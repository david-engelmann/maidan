### Fixed

- **Fixed:** Egress refuses documentation and special-purpose IPv6, and a listed peer may not be unspecified, multicast, broadcast, or link-local, including those addresses inside NAT64. Resume tokens are size-checked and MAC-checked before the payload is decoded. Export verification rejects a weak Ed25519 key. A federation peer of a suspended workspace is refused, bearer secrets are length-bounded, and secret debug output is redacted. Broadcast channels are capacity-bounded, resource notifications are chunked in one pass, and dropping a Postgres bus or notifier stops its listener.
