### Advisories: crypto-bigint, fastmcp, undici

- **Fixed:** the S3 client drops the `sigv4a` feature, which nothing used and which pulled crypto-bigint 0.7.5 (RUSTSEC-2026-0387) into the root and fuzz lockfiles. crypto-bigint 0.5.5 remains through p256 0.13 (OpenID Connect verification and web push), has no fixed release, and is ignored in `deny.toml` with the reason. `fastmcp` is floored at 3.4.10 in the OpenHands recipe, and `undici` is overridden to 7.30.1 in the Cloudflare deploy tooling.
