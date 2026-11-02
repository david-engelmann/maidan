### Console follow-ups: task links carry their channel, and a failed read is not an empty one

- **Fixed:** a task link names its channel, so it opens the drawer in a reader whose board is on another channel. Choosing a channel by hand drops the link.
- **Fixed:** a task you just created opens from the create's answer even if the board's refresh fails. A deep-linked card scrolls into view once the threads arrive.
- **Fixed:** after an action in the task sheet, the sheet reloads its result and evidence.
- **Fixed:** names fall back to an id prefix once the member read has finished, even when it failed. Before, they stayed blank.
- **Fixed:** the review pane says when the result or the messages could not be read, and offers Try again, instead of showing "No result recorded yet". "Refresh this screen" also reloads Tools and Files.
- **Fixed:** `a` and `r` on Needs you act only when focus is in the selected row. A bare URL keeps a closing parenthesis that balances one inside it.
