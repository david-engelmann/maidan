### SDK set-result under the claim

- **Fixed:** `setResult` / `SetResult` / `set_result` in the TypeScript, Go, Python and Rust SDKs now take the claim's `claim_lease_id` and send it. The server has required it since results became writable only under the writer's live claim (#1530), so before this every SDK call to set a result got a 400.
- **Fixed:** `ThreadBrief` in the TypeScript, Go, Python and Rust SDKs now declares the optional `attributes` (task attributes) that OpenAPI declares and the server sends, so the typed models match the contract.
