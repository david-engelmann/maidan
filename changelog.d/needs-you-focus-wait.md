### The redrawn-row focus test waits for focus to settle

- **Fixed:** `needs-you-focus.spec.ts` "a redrawn row never hands focus to a disabled control" read the focused element once, right after the redraw, and failed whenever focus moved a moment later. It now polls until focus settles, the fix Open Work recorded for this flake on 2026-10-23.
