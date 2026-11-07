### Live resume, push notifications and toasts stay in their own place

- **Fixed:** a WebSocket resume token is kept with the workspace that issued it, so switching workspaces or reloading into another one no longer sends the previous workspace's token.
- **Fixed:** a Web Push `log_id` sent as a number is kept, the server's `New notification (kind)` text is shown in words, and a click opens the thread, the inbox or Needs you, replacing a duplicate notification for the same event.
- **Fixed:** a secret shown once starts hidden again when a new secret replaces it, and "Copied" clears on its own. Error toasts announce immediately, and a toast waits while the pointer or focus is on it.
