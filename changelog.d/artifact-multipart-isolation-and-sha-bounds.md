### Fixed

- **Fixed:** A multipart upload is named under the caller's workspace, and abort, complete, and part upload accept only that temporary key. An empty upload body is refused on MCP the same way it is on REST. SHA-256 hex rejects a leading `+`, compares in constant time, and an uppercase digest still reads the artifact.
