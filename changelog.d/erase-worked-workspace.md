### A workspace that has done work can be erased on Postgres

- **Fixed:** erasing a workspace on Postgres answered 500 once a thread in it
  had a transition, a result, reported usage, a review packet or an invite.
  Those rows name a member through a foreign key that does not cascade, and
  Postgres deletes a workspace's members before it reaches its threads. The
  erase now deletes those rows first, reading the keys from the catalog, so a
  table added later with such a key cannot break it again. The launch gate's
  cleanup hit it on maidan.world; SQLite was not affected.
- **Fixed:** the console's task sheet took focus back to its close button
  whenever the board re-rendered (every 30 seconds, and on every refresh and
  live event). That shut an open "More actions" menu and pulled the caret out
  of a note being typed.
- **Changed:** when a console step of the launch gate fails, the evidence
  holds a screenshot of the page as it stood, and the line says what the step
  was waiting for.
