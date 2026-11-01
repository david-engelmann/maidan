### Evidence records on a task

- **Added:** A task can carry evidence records: a test run, a CI run, a diff, a review verdict, a screenshot by URL, or a note, each with its source, who recorded it and when, and an optional URL, sha256 and note. `POST /threads/{id}/evidence-records` (`workspace:write`) adds one and `GET` (`workspace:read`) lists them, with the MCP twins `add_evidence_record` and `list_evidence_records`. Each record publishes `evidence_added`, and `GET /threads/{id}/evidence` merges the records into the timeline.
