import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Filenames stay stable (app.js, vendor.js, story.js, style.css) so the
// server can embed them without a manifest.
export default defineConfig({
  base: "/landing/",
  plugins: [react()],
  build: {
    outDir: process.env.LANDING_OUT ?? "../../crates/maidan-server/static/landing/dist",
    emptyOutDir: true,
    assetsDir: "",
    cssCodeSplit: false,
    modulePreload: false,
    rollupOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "[name].js",
        assetFileNames(info) {
          const name = info.names?.[0] ?? "";
          if (name.endsWith(".css")) return "style.css";
          if (name.endsWith(".woff2")) return "geist.woff2";
          return "[name][extname]";
        },
        manualChunks(id) {
          if (id.includes("node_modules")) return "vendor";
          if (id.includes("/src/story")) return "story";
          if (id.includes("/src/aurora")) return "aurora";
        },
      },
    },
  },
});
