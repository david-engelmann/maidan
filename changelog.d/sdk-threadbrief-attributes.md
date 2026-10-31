### SDK ThreadBrief attributes

- **Fixed:** `ThreadBrief` in the TypeScript, Go, Python and Rust SDKs now declares the optional `attributes` (task attributes) that OpenAPI declares and the server sends, so the typed models match the contract.
- **Fixed:** `Thread` in all four SDKs now declares `ci` (a list of the new `PullCi`, with `MergeQueue` and `ReviewThreadLink`) and `withheld` (the new `ThreadWithheld`), and `TaskAttributes` declares `author_family`, matching OpenAPI, so sdk interop sees no undeclared members.
