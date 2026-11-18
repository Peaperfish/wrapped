import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri loads http://localhost:5173 in `tauri dev` and ../ui/dist when built.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: "safari16",
    outDir: "dist",
    emptyOutDir: true,
  },
});
