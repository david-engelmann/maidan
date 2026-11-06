### Fixed

- **Fixed:** The try/demo Cloudflare edge fails closed on oversized or sneaky uploads (`Transfer-Encoding`, missing or non-decimal `Content-Length` on body methods), normalizes and decodes scanner pathnames so encoded `/.env` and `/./.env` probes never wake a container, expands common credential/actuator probe patterns, and hardens the try-page Turnstile mint (control-bearing tokens rejected, host match case-insensitive, trailing-slash asset and `/sandbox/` paths resolved).
