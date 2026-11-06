### Fixed

- **Fixed:** Go SDK cache/usage helpers reject blank or whitespace-only ids and providers and ignore `__proto__`/`prototype`/`constructor` usage path keys, and the console OpenAPI type generator skips unsafe `$ref`s and path templates with empty or parent segments so generated `ApiRoute` entries stay fail-closed.
