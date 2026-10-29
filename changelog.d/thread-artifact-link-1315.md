### A file attached to a task is linked as evidence

- **Fixed:** Uploading a file onto the open task posted a message that cited the hash and did not link the artifact to the task. Review and the evidence timeline only read those links, so the file never became evidence. The console now links it with `PUT /ui/api/threads/{id}/artifacts/{sha}` before posting, lists the task's linked files, and can unlink one.
