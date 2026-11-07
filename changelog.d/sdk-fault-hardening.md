### A lost answer is retried instead of taken twice

- **Fixed:** The TypeScript, Go and Rust clients read a response to the end inside the retry loop. A claim whose body is cut short, or a 2xx that is not JSON, is retried with the same idempotency key. A claim missing its id or lease is an error the caller can catch, and a transport that never answers is a status-0 error rather than a silent short read.
