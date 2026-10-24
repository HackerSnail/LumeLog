import { defineConfig } from "vite";

// Tauri expects a fixed dev port (spec: 1420). `strictPort` fails loudly rather
// than drifting to another port the shell config wouldn't know about.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "es2021",
  },
});
