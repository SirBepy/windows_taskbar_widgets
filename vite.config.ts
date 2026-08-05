import { defineConfig, Plugin } from "vite";
import { resolve } from "path";

// Phosphor's @font-face lists woff2/woff/ttf/svg, so all four get emitted (4.1MB,
// 3.0MB of it the svg) into every installer and updater download. WebView2 is
// Chromium and only ever loads the woff2. Must run before Vite resolves the
// url()s, or the files are emitted regardless.
function woff2Only(): Plugin {
  return {
    name: "phosphor-woff2-only",
    enforce: "pre",
    transform(code, id) {
      if (!id.endsWith(".css") || !code.includes("Phosphor")) return null;
      const stripped = code.replace(
        /src:\s*[^;}]*?url\(([^)]*\.woff2[^)]*)\)[^;}]*/g,
        (_m, woff2) => `src: url(${woff2}) format("woff2")`,
      );
      if (stripped === code) this.error(`expected a woff2 @font-face src in ${id}`);
      return stripped;
    },
  };
}

export default defineConfig({
  plugins: [woff2Only()],
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
