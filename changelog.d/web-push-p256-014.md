### web_push on p256 0.14; corrected RUSTSEC-2026-0387 ignore

- **Fixed:** Web Push (VAPID signing and payload encryption) now uses p256 0.14, which pulls the fixed crypto-bigint 0.7.x, so the long-lived VAPID key no longer runs on crypto-bigint 0.5.5. The RUSTSEC-2026-0387 ignore now names the right fix version (0.7.6) and covers only openidconnect's public-key verification (#1596).
