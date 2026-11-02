### A public status page on the hosted Worker

- **Added:** The Cloudflare Worker serves `/status`, a plain page drawn from the container's `/health` that says whether the database, artifact storage, live updates and search index are working, says plainly when the container does not answer, and never shows the server's error text.
