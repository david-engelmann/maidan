### The landing's first paint is right and lighter

- **Fixed:** the server writes whether anyone may sign up into the landing page (`<meta name="maidan-signup">`), so the first paint already says "Create a workspace" on maidan.world. Before, it said "Join the waitlist" until a discovery request answered. The footer no longer says "Your email is used only for your invite": that is the invite sheet's promise, and the sheet carries it.
- **Changed:** the invite sheet, and the `motion` library that only it uses, load only on an instance that signs people up by invite (`join.js` and `motion.js`). On maidan.world, with open sign-up, they never load. The landing's first JavaScript is 98 KB gzipped, where it was 139 KB.
