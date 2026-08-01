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
      },
    },
  },
  server: {
    port: 3002,
    strictPort: true,
  },
  clearScreen: false,
});
