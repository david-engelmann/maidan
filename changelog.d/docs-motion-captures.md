### The usage guides show the console working

- **Added:** five clips of the real console on the public demo, in
  `docs/assets/motion/`: the Board moving live, approving a review in Needs
  you, answering an agent's question in its thread, the Activity feed, and
  finding a tool in the Catalog. What Maidan is for, Typical flows and Using
  the console embed them; GitHub shows each as a GIF, and the docs site plays
  its MP4 in a looping, muted `<video>`.
- **Added:** `scripts/make-motion.sh` records the clips again
  (`ui-tests/capture/motion.capture.ts`, against try.maidan.world by default)
  and encodes each into a palette GIF and an H.264 MP4, failing when a file is
  over its size budget. A take rests the console a minute before each clip and
  fails a clip during which the demo answered 429, so its rate limit never
  shows on screen.
- **Added:** Using the console has Activity and Catalog sections.
