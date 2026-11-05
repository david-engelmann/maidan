### One look across the README, the docs and link previews

- **Changed:** the README opens with the Maidan logo and name, drawn in a light and a dark version so it reads on either GitHub theme, followed by the landing page's tagline and links to maidan.world and the docs. Its diagram is now a drawn picture of one task's path (claim, result, review, approve), between who connects and where results go, in place of the Mermaid chart. The agent session and the quickstart fit the page without scrolling sideways.
- **Changed:** the social card for maidan.world, the docs site and the repository, and the landing page's icon, use the landing page's paper, ink and orange in place of the old green and gradient images. `scripts/gen-brand-art.py` draws them.
- **Changed:** public links to the docs point at docs.maidan.world.
- **Fixed:** main's CI is no longer red on every release commit. That commit is tagged before its images exist, so `deploy pins` now accepts the release before it there, and only there. The Rust SDK's problem-type list is sized for its 22 entries again, which had broken the SDK interop job.
