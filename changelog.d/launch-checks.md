### Two things the release checks found

- **Fixed:** an open task's sheet, and the review pane under Needs you, kept showing "No result recorded yet." and "No messages yet." after an agent set a result or posted, until the task was opened again. Both now read again when the live stream says the task's result or messages changed.
- **Fixed:** `scripts/mcp-inspector.sh`, and the `mcp inspector` job with it, could not start its server since the provider address had to be `https`: its stand-in provider listens on `127.0.0.1`. `MAIDAN_JEV_BASE_URL` may be plain `http` to a loopback address listed in `MAIDAN_PRIVATE_EGRESS_HOSTS`, where the key crosses no network; any other private peer still needs `https`.
- **Fixed:** an outbound target written as an IPv6 literal (`https://[2606:4700:4700::1111]/hook`) never resolved: the bracketed text went to the system resolver as a name. The brackets are stripped for the lookup.
