### The console says loading, empty and failed the same way everywhere

- **Added:** one data-state primitive (`shell/DataState.tsx`): a read is loading, empty, failed or ready, decided by one rule (rows on screen outrank a later failure, a failure outranks loading, loading outranks empty), with the "Could not load …" sentence in one place.
- **Fixed:** Activity said "Nothing has happened yet" while its first read was still in flight; it now shows skeleton rows until the read answers.
- **Fixed:** a place no longer fades in from opacity 0, so its skeletons and empty states show from the first frame.
- **Fixed:** People showed the browser's own "Failed to fetch" when the server was unreachable; its reads now use the shared offline sentence like every other screen.
- **Fixed:** the invite form showed to a non-admin whenever the invites read failed with anything but 403. The form now shows only once the list has loaded, a failure offers Try again, and a token without `workspace:admin` is never sent the request (no failed request in the browser console).
- **Fixed:** a member's work list counted channels whose tasks failed to load as empty; it now names them above the list.
- **Changed:** the empty Threads, People and Catalog detail panes show the place's own icon instead of a blank steel square, and the empty Threads pane links to the board. An empty Needs you says so once, in the caught-up card, not also as "Nothing waiting" above it.
