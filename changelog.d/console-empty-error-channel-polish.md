### Fixed

- **Fixed:** Console empty, load-error, and channel-picker states stay calm and accessible: blank empty titles and load messages fall back instead of rendering hollow copy, empty workspaces say “No channels yet” instead of a disabled blank select, a remembered channel id that left the list clears the menu as an orphan with `aria-invalid`, EmptyState/LoadError announce via `aria-live`, `useSession` passes a `getServerSnapshot`, reduced-motion reads survive a throwing `matchMedia`, and a missing `#root` logs instead of mounting nothing.
