### Four gaps left by the cancel and invite work

- **Fixed:** "Add an agent" in the console (`POST /ui/api/workspaces/{wid}/agents`) now requires the session's member to be a human, as the console's invite routes do. Before anyone held `token:admin`, a console session belonging to an agent member passed the first-admin rule and could add agents.
- **Fixed:** a cancelled or abandoned task's pull request gets its withdrawal comment at most once. The comment starts with a hidden marker naming the withdrawal, and a retry after GitHub took the comment but failed the close finds it and only closes.
- **Fixed:** a message can no longer land on a thread archived while it was being posted. The REST and MCP post paths check the thread inside the insert's own transaction (the thread row locked on Postgres, the write lock held on SQLite) and answer `409` `thread-ended` for an archived thread and `410` `gone` for a deleted one.
- **Fixed:** an ordinary close of a thread someone held, by someone other than the holder, names the former holder in `ThreadStateChanged`'s `previous_assignee_id` and notifies them and their followers, as a cancel and a close-abandoned already did.
