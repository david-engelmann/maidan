import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The console is served at /console by the Rust server, which embeds these
// files by name. Filenames stay stable (app.js, vendor.js, style.css,
// geist.woff2) so no manifest is needed. Nothing is inlined: the strict CSP
// allows no inline script and no <style> element, and scripts/check-csp.mjs
// fails the build if either shows up in index.html.
export default defineConfig({
  base: "/console/",
  plugins: [react()],
  server: {
    // `npm run dev` talks to a local server for the API and the socket.
    proxy: {
      "/ui/api": "http://127.0.0.1:8080",
      "/v1": "http://127.0.0.1:8080",
      "/ws": { target: "ws://127.0.0.1:8080", ws: true },
      "/.well-known": "http://127.0.0.1:8080",
    },
  },
  build: {
    outDir: process.env.CONSOLE_OUT ?? "../../crates/maidan-server/static/console/dist",
    emptyOutDir: true,
    assetsDir: "",
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    // Preload the chunks app.js imports, without the polyfill (every target
    // browser has modulepreload). scripts/check-csp.mjs holds the build to it.
    modulePreload: { polyfill: false },
    rollupOptions: {
      // The service worker is its own entry so it lands at /console/sw.js,
      // where its scope covers the whole console.
      input: { app: "index.html", sw: "src/pwa/sw.ts" },
      output: {
        entryFileNames: (chunk) => (chunk.name === "sw" ? "sw.js" : "app.js"),
        chunkFileNames: "[name].js",
        assetFileNames(info) {
          const name = info.names?.[0] ?? "";
          if (name.endsWith(".css")) return "style.css";
          if (name.endsWith(".woff2")) return "geist.woff2";
          return "[name][extname]";
        },
        manualChunks(id) {
          if (id.includes("node_modules")) return "vendor";
        },
      },
    },
  },
});
