import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

// Overlay-Frontend für Tauri. /shared liegt außerhalb von ui/ und wird mitgebaut.
export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  clearScreen: false,
  server: { port: 5173, strictPort: true, fs: { allow: ["..", "../../shared"] } },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    assetsInlineLimit: 0,
    // Zwei Seiten: Overlay und Einstellungen/Diagnose (gemeinsame Teile werden geteilt)
    rollupOptions: { input: { overlay: "index.html", settings: "settings.html", setup: "setup.html" } },
  },
});
