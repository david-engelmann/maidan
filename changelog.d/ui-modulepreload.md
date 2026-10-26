### Board modules preload

- **Changed:** The board page (`/ui/`) preloads every module `main.js` reaches with `<link rel="modulepreload">`, so the browser requests them in parallel instead of finding them one import level at a time. The list is generated from the import graph (`make regen`), and a contract test fails if it drifts or names a module the server doesn't serve.
