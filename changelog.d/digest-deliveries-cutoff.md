### The email digest honors the deliveries cutoff and is metered

- **Fixed:** the digest sweeper sends no digest to a member whose workspace is past its deliveries allowance, leaving the watermark so the digest goes out once the cutoff lifts, and counts one delivery per digest sent (#1644).
