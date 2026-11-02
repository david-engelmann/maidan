### Console board: task drawer, filters and new task

- **Added:** Opening a task on the board shows a side drawer with its state, holder, result, latest messages, linked evidence and actions. Esc closes it and the URL deep-links to it.
- **Added:** A filter bar on the board for agent, state and free-text search, kept in the URL.
- **Added:** A New task composer, opened with `c` or the button, that creates a task in the current channel.
- **Changed:** The task drawer on a wide screen sits beside the board, which narrows to make room so the open card stays visible and highlighted, and other cards stay clickable. Opening a card no longer scrolls the page away from the filter bar. It shows state with the board's status dot, the holder with initials, and the actions that fit the state up front (Claim, Send to review, Approve, Send back), with Close, Archive and Record a result in a More actions menu.
- **Fixed:** A sign-in rejected for an older credential no longer signs out the new one, and the palette's Sign out ends the server session the same way the Session card does.
- **Fixed:** File buttons are disabled while an erase is in flight, the palette ranks title matches above hint matches, and focus rings, disabled buttons, dark-mode toasts and the channel picker read correctly.
