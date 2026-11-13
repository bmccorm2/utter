import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  // Keep Rust errors visible in `tauri dev`.
  clearScreen: false,
  server: {
    // Tauri expects a fixed port.
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "safari17",
    rollupOptions: {
      input: { main: "index.html", overlay: "overlay.html" },
    },
  },
});
