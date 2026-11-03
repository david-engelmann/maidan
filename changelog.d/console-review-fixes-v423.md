### Console: review fixes for the board, Activity, Files and Catalog

- **Fixed:** Activity offers Try again when its first read of recent events fails, instead of an error above "No events yet". A member freeze or unfreeze names who did it, not the frozen member.
- **Fixed:** In People, a member's work list labels a task waiting on an approval gate as the Board does, and its ages keep moving while the panel is open.
- **Fixed:** The Board says "Nothing here matches." when a health chip hides every task in a lane or in the By agent view, and warns when live updates never connect.
- **Fixed:** Vote text and badges reach 4.5:1 contrast in dark mode, and a freshly updated card keeps its ring in dark mode.
- **Fixed:** Files labels a CSV file as Data, reports a failed read of a task's links with Try again instead of leaving its files out, says when an image's bytes are missing, and reloads a thumbnail after switching from List back to Grid.
- **Fixed:** Catalog shows its load error and Try again on a phone when a tool is open. A tool's hint badges wrap so its title keeps its room.
- **Fixed:** `j` and `k` move through every list row in Tools, Artifacts and the chooser.
- **Fixed:** `npm run check:spec` also checks single-quoted paths and calls under `/tools` and `/ui/api`.
