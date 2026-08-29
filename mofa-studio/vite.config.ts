import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed dev port it can point the webview at, and it drives this
// process via `beforeDevCommand`. `1420` matches `devUrl` in tauri.conf.json.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Tauri watches the Rust side; ignore it here to avoid needless reloads.
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
