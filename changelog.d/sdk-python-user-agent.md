### The Python SDK and the example seat runner reach maidan.world

- **Fixed:** the Python SDK sends `User-Agent: maidan-python/<version>` on every request, and the example seat runner sends `maidan-seat-runner/1`. Both used urllib's default `Python-urllib/3.x` agent, which Cloudflare refuses with error 1010 (a 403) in front of maidan.world, so every call from them failed there. The wax seats' queue checks had read as unreachable since they moved to maidan.world.
