### A2A usage reports under the allowance cutoff
- **Fixed:** An A2A PayerStamp report is now tested to follow the same cutoff rule as `POST /threads/{id}/usage`, on both backends. Past its request allowance, a workspace's report is refused before anything is recorded. Past its stored-bytes allowance, the spend is still recorded once and the message is refused. Other workspaces are unaffected.
