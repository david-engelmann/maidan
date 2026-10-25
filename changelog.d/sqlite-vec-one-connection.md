### SQLite semantic search no longer 500s on an older pooled connection

- **Fixed:** SQLite semantic search now loads and checks sqlite-vec on the connection it runs its query on, so a pooled connection opened before the extension was registered no longer fails with "no such function: vec_distance_cosine" (HTTP 500).
