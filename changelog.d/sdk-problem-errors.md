### Every problem type has its own SDK error

- **Added:** the four SDKs raise a dedicated error for `unprocessable-entity`, `media-quota-exceeded`, `footprint-conflict`, `plan-conflict` and `claim-lost` (for example `ClaimLostError` in TypeScript and Python, `*maidan.ClaimLostError` in Go, `MaidanError::ClaimLost` in Rust), where they arrived as the unknown-problem error before. Each SDK's test now holds its map to all 22 types the server sends.
