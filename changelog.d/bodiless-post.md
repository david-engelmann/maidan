### A POST whose body is optional takes no body

- **Changed:** Claiming or unassigning a thread, claiming the next thread, freezing a member and starting an embeddings reindex accept a POST with no body and no `Content-Type`, read as `{}`. They answered 415 before. A body that is sent is still read as JSON.
