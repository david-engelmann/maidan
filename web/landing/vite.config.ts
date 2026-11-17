import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Filenames stay stable (app.js, vendor.js, style.css, geist.woff2) so the
// server can embed them without a manifest.
export default defineConfig({
  base: "/landing/",
  plugins: [
    react(),
    // Dev only: the server owns /.well-known/maidan.json; answer it locally so
    // the dev console stays clean (no Turnstile key, so the widget stays off).
    {
      name: "dev-well-known",
      apply: "serve",
      configureServer(server) {
        server.middlewares.use("/.well-known/maidan.json", (_req, res) => {
          res.setHeader("content-type", "application/json");
          res.end("{}");
        });
      },
    },
  ],
  build: {
    outDir: process.env.LANDING_OUT ?? "../../crates/maidan-server/static/landing/dist",
    emptyOutDir: true,
    assetsDir: "",
    cssCodeSplit: false,
    modulePreload: false,
    rolldownOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "[name].js",
        assetFileNames(info) {
          const name = info.names?.[0] ?? "";
          if (name.endsWith(".css")) return "style.css";
          if (name.endsWith(".woff2")) return "geist.woff2";
          return "[name][extname]";
        },
        // Dependencies in vendor.js. Rolldown puts the helpers both chunks
        // share in rolldown-runtime.js, which the server embeds beside them.
        // motion goes in motion.js, loaded with the invite sheet (join.js),
        // so the first paint carries React alone. The room's drawing is its
        // own chunk too (roomviz.js), loaded as its section nears the screen.
        codeSplitting: {
          // A group takes only the modules its test names: left to pull in
          // their dependencies too, the motion group took React's JSX runtime
          // and app.js had to load motion.js to render anything.
          includeDependenciesRecursively: false,
          groups: [
            { name: "motion", test: /node_modules[\\/](motion|motion-dom|motion-utils|framer-motion)[\\/]/, priority: 2 },
            { name: "vendor", test: /node_modules/, priority: 1 },
          ],
        },
      },
    },
  },
});
