import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Configuration attendue par Tauri : port fixe, pas d'écran d'erreur masquant les logs Rust.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "es2022", sourcemap: false },
});
