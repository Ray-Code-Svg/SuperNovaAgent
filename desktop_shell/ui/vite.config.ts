import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: false
  },
  build: {
    outDir: "dist",
    emptyOutDir: true
  },
  test: {
    environment: "node"
  }
});
