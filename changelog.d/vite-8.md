### The console and landing build with Vite 8

- **Changed:** The console and the landing build with Vite 8.3 (Rolldown) and `@vitejs/plugin-react` 6.3, in place of Vite 7. The console's bundle is about 2 KB smaller gzipped. The helpers that `app.js` and `vendor.js` share are now their own file, `rolldown-runtime.js`. The server serves it for both apps beside the other pinned names, and the console's offline shell caches it.
