### API errors link to maidan.world, where each problem type is explained

- **Fixed:** every problem `type` was `https://maidan.dev/problems/<name>`, a domain Maidan does not own, so each API error linked a stranger's site. Types are now `https://maidan.world/problems/<name>`, in the server, the OpenAPI document and all four SDKs. This changes the wire value, so clients that compare whole `type` strings must update; the SDKs compare against their new base.
- **Added:** `maidan.world/problems` explains every type: what happened and what to do. A type's URI opens its row there. `scripts/check-our-domains.sh` (in `lint`) fails on any link to `maidan.dev` outside history, and the Worker's tests hold the page's rows to the server's list of types.
- **Changed:** a browser that follows a bad link on maidan.world gets a not-found page instead of the server's JSON body. API clients still get `application/problem+json`.
- **Changed:** the k8s overlays label resources `maidan.world/environment`, replacing `maidan.dev/environment`.
