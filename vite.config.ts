import { defineConfig } from "vite";
import { resolve } from "path";

export default defineConfig({
  root: "src",
  publicDir: false,
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, "src/index.html"),
        flyout: resolve(import.meta.dirname, "src/flyout.html"),
        dashboard: resolve(import.meta.dirname, "src/dashboard.html"),
      },
    },
  },
  server: {
    // Must match tauri.conf.json's devUrl. Moved off 3002 in Aug 2026: zng-api's
    // payment service binds it, and strictPort makes that collision fatal.
    port: 3102,
    strictPort: true,
  },
  clearScreen: false,
});
